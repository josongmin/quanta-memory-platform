use std::marker::PhantomData;
use std::sync::Mutex;

use quanta_memory_platform_core::{MemoryError, PublishError, StagedHandle, StoreBindingHandle};

use crate::publish_pipeline::PublishPipeline;

/// A caller-supplied commit transition: given the staged input and the commit
/// token, either produce the published value or fail.
///
/// This is the only place a `Published` value can be constructed, which is what
/// structurally guarantees the pipeline never exposes half-published state: a
/// staged-but-not-committed value has type `StagedHandle<Stage>`, never the
/// `Published` type, and a failed transition produces an `Err`, never a
/// half-built `Published`.
pub trait CommitTransition<Stage, Commit, Published> {
    fn apply(&self, staged: Stage, commit: Commit) -> Result<Published, PublishError>;
}

impl<Stage, Commit, Published, F> CommitTransition<Stage, Commit, Published> for F
where
    F: Fn(Stage, Commit) -> Result<Published, PublishError>,
{
    fn apply(&self, staged: Stage, commit: Commit) -> Result<Published, PublishError> {
        (self)(staged, commit)
    }
}

/// Portable legacy staged-publish transition.
///
/// New retaining publication integrations use
/// [`crate::publish_pipeline::PreparedPublicationService`]. This type remains
/// only for existing non-retaining transition callers; it owns no visibility
/// or storage authority and is scheduled for removal by the QMP publish-pipeline
/// migration owner.
///
/// Owns the stage -> commit -> publish transition skeleton (playbook 5.2):
/// `stage` reserves a staged handle, `commit` runs the caller's commit transition,
/// and only a successful transition yields a published value. The `published_count`
/// counter increments solely inside a committed transition, so the count never
/// reflects a half-published item.
///
/// Accounting note: this pipeline is a *non-retaining transition*, not a store —
/// it transforms a staged value into a published one and hands it off. It holds no
/// bytes and therefore does NOT charge the ledger; the destination store that
/// retains the published value (payload / retained-map / transcript) is what
/// charges those bytes against the broker, so wiring a charger here would
/// double-count. This is intentional, not a missing integration.
pub struct SimplePublishPipeline<Stage, Commit, Published, T = DefaultCommitTransition> {
    binding: StoreBindingHandle,
    transition: T,
    published_count: Mutex<u64>,
    _marker: PhantomData<(Stage, Commit, Published)>,
}

impl<Stage, Commit, Published, T> std::fmt::Debug
    for SimplePublishPipeline<Stage, Commit, Published, T>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The commit transition is not necessarily Debug (e.g. a closure), so we
        // print the stable, observable fields only.
        f.debug_struct("SimplePublishPipeline")
            .field("binding", &self.binding)
            .field("published_count", &self.published_count.lock().map(|c| *c).ok())
            .finish_non_exhaustive()
    }
}

impl<Stage, Commit, Published>
    SimplePublishPipeline<Stage, Commit, Published, DefaultCommitTransition>
where
    Published: From<Stage>,
{
    /// Build a pipeline whose commit transition is the infallible identity
    /// `Published: From<Stage>` conversion.
    #[deprecated = "legacy staging pipeline: prefer the prepared-publication service; this constructor will be removed with the legacy transition trait"]
    pub fn new(binding: StoreBindingHandle) -> Self {
        Self {
            binding,
            transition: DefaultCommitTransition,
            published_count: Mutex::new(0),
            _marker: PhantomData,
        }
    }
}

impl<Stage, Commit, Published, T> SimplePublishPipeline<Stage, Commit, Published, T> {
    /// Build a pipeline with an explicit, possibly-fallible commit transition.
    #[deprecated = "legacy staging pipeline: prefer the prepared-publication service; this constructor will be removed with the legacy transition trait"]
    pub fn with_transition(binding: StoreBindingHandle, transition: T) -> Self {
        Self { binding, transition, published_count: Mutex::new(0), _marker: PhantomData }
    }

    pub fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    /// Number of values that have actually been published (committed). Never
    /// counts a staged-but-uncommitted value.
    #[deprecated = "legacy staging pipeline surface: prefer the prepared-publication service"]
    pub fn published_count(&self) -> Result<u64, MemoryError> {
        self.published_count
            .lock()
            .map(|guard| *guard)
            .map_err(|_| MemoryError::poisoned("publish pipeline lock"))
    }
}

/// The default identity transition used by [`SimplePublishPipeline::new`].
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultCommitTransition;

impl<Stage, Commit, Published> CommitTransition<Stage, Commit, Published>
    for DefaultCommitTransition
where
    Published: From<Stage>,
{
    fn apply(&self, staged: Stage, _commit: Commit) -> Result<Published, PublishError> {
        Ok(Published::from(staged))
    }
}

impl<Stage, Commit, Published, T> PublishPipeline<Stage, Commit, Published>
    for SimplePublishPipeline<Stage, Commit, Published, T>
where
    T: CommitTransition<Stage, Commit, Published>,
{
    fn binding(&self) -> &StoreBindingHandle {
        &self.binding
    }

    fn stage(&self, input: Stage) -> Result<StagedHandle<Stage>, PublishError> {
        // Staging produces only a StagedHandle; it is never visible as Published.
        Ok(StagedHandle::new(input))
    }

    fn commit(
        &self,
        staged: StagedHandle<Stage>,
        commit: Commit,
    ) -> Result<Published, PublishError> {
        // Run the transition FIRST. If it fails, nothing is published and the
        // counter is untouched: no half-published state escapes.
        let published = self.transition.apply(staged.staged, commit)?;
        let mut count = self
            .published_count
            .lock()
            .map_err(|_| MemoryError::poisoned("publish pipeline lock"))?;
        *count = count.saturating_add(1);
        Ok(published)
    }
}

#[cfg(test)]
#[allow(deprecated)] // Legacy-path tests remain until the QMP publish-pipeline removal owner deletes it.
mod tests {
    use super::*;
    use quanta_memory_platform_core::{MemoryClass, StoreBindingHandle};

    fn binding() -> StoreBindingHandle {
        StoreBindingHandle::new(MemoryClass::ManifestResidency, "owner-pub".to_string(), None)
    }

    #[test]
    fn identity_pipeline_stages_then_commits_published() {
        let pipeline: SimplePublishPipeline<String, (), String> =
            SimplePublishPipeline::new(binding());
        // Staging yields a StagedHandle, never the Published type.
        let staged = pipeline.stage("v1".to_string()).expect("stage");
        assert_eq!(staged.staged, "v1");
        // Nothing published until commit.
        assert_eq!(pipeline.published_count().expect("count before"), 0);

        let published = pipeline.commit(staged, ()).expect("commit");
        assert_eq!(published, "v1");
        assert_eq!(pipeline.published_count().expect("count after"), 1);
    }

    #[test]
    fn failed_commit_never_publishes_and_never_increments_count() {
        // NEGATIVE-PATH 3: staged publish fails after reservation. A failing commit
        // transition must yield Err and never bump the published count, so a
        // half-published value can never be observed.
        let transition = |staged: String, _commit: ()| -> Result<String, PublishError> {
            if staged == "poison" {
                Err(MemoryError::capacity("commit rejected: staged publish failed"))
            } else {
                Ok(staged)
            }
        };
        let pipeline: SimplePublishPipeline<String, (), String, _> =
            SimplePublishPipeline::with_transition(binding(), transition);

        let staged = pipeline.stage("poison".to_string()).expect("stage");
        let err = pipeline.commit(staged, ()).expect_err("commit must fail");
        assert!(
            matches!(err, MemoryError::Capacity(_)),
            "failed commit must surface the typed error, got {err:?}"
        );
        assert_eq!(
            pipeline.published_count().expect("count after failure"),
            0,
            "failed commit must not publish: no half-published state"
        );

        // Positive correction: a non-poison value commits and increments the count.
        let staged_ok = pipeline.stage("ok".to_string()).expect("stage ok");
        let published = pipeline.commit(staged_ok, ()).expect("commit ok");
        assert_eq!(published, "ok");
        assert_eq!(pipeline.published_count().expect("count ok"), 1);
    }

    #[test]
    fn default_commit_transition_applies_identity_with_no_missing_step() {
        // scenario F: DefaultCommitTransition stages apply with no missing/rolled-back
        // step. Drive the default identity transition directly: it must produce the
        // From<Stage> value and never error (no rollback path on the happy commit).
        let transition = DefaultCommitTransition;
        // Stage type String -> Published type String via From (identity).
        let published: String =
            transition.apply("staged-value".to_string(), ()).expect("identity transition applies");
        assert_eq!(
            published, "staged-value",
            "identity transition must pass the staged value through"
        );
    }

    #[test]
    fn default_pipeline_runs_stage_then_commit_each_exactly_once_no_rollback() {
        // scenario F: the default pipeline stage -> commit sequence applies every
        // step with none missing and none rolled back. Each commit advances the
        // published count by exactly one; staging alone advances nothing.
        let pipeline: SimplePublishPipeline<String, (), String> =
            SimplePublishPipeline::new(binding());

        for (i, v) in ["x", "y", "z"].iter().enumerate() {
            let staged = pipeline.stage((*v).to_string()).expect("stage");
            // Staging is a distinct step that does NOT publish (no premature step).
            assert_eq!(pipeline.published_count().expect("staged-only"), i as u64);
            let published = pipeline.commit(staged, ()).expect("commit");
            // Commit applies the identity transition and increments once (no rollback).
            assert_eq!(published, *v);
            assert_eq!(pipeline.published_count().expect("after commit"), (i + 1) as u64);
        }
        // All three steps applied, none rolled back.
        assert_eq!(pipeline.published_count().expect("final"), 3);
    }

    #[test]
    fn published_count_only_reflects_committed_not_staged() {
        let pipeline: SimplePublishPipeline<u32, (), u32> = SimplePublishPipeline::new(binding());
        // Stage three values but commit only one.
        let s1 = pipeline.stage(1).expect("s1");
        let _s2 = pipeline.stage(2).expect("s2");
        let _s3 = pipeline.stage(3).expect("s3");
        assert_eq!(pipeline.published_count().expect("none committed"), 0);
        pipeline.commit(s1, ()).expect("commit s1");
        assert_eq!(
            pipeline.published_count().expect("one committed"),
            1,
            "only committed values count as published"
        );
    }
}
