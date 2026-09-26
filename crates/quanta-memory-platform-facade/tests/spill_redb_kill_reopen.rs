//! Real forced-termination / reopen rail for the durable spill tier.
//!
//! Boundary proved here: the child process is ended with
//! [`std::process::Child::kill`] — `SIGKILL` on Unix, which runs no
//! destructors and flushes nothing — and a survivor reopens the SAME database
//! file cold. This is strictly stronger than the in-module drop/reopen
//! simulation (same process, descriptors closed cleanly) and the injected
//! crash-point tests (faults at write boundaries, same address space).
//!
//! Boundary NOT proved here: power loss. A `SIGKILL` rail cannot fail the
//! storage device mid-sector-write, so power-loss durability remains
//! unverified by this rail.
//!
//! Backing-only cases drive recovery through the public
//! [`ReconciledDurableSpillTarget`] / [`DurableRestoreTarget`] surface. The
//! `revocable_store` case also drives the public platform/store builders and
//! charged restore path.
//! The exercised interruption boundaries are explicit:
//!
//! - after committed records plus a persisted restore claim;
//! - while a child continuously issues reserve/commit operations (the exact
//!   scheduler point relative to an individual redb transaction is unknown);
//! - after a reserved-but-never-committed record ID.
//!
//! This rail does not prove an interrupt inside a specific redb transaction or
//! at the ACK, source-release, restore-delete, or finalize boundaries.
//! Run with:
//!
//! ```bash
//! cargo test --locked -p quanta-memory-platform-facade --features spill-redb \
//!     --test spill_redb_kill_reopen
//! ```

#![cfg(feature = "spill-redb")]

use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

#[cfg(feature = "revocable_store")]
use quanta_memory_platform_core::{MemoryClass, MemoryPreset, StoreBindingHandle};
use quanta_memory_platform_facade::spill::backings::redb::RedbSpillBacking;
use quanta_memory_platform_facade::spill::service::{
    DurableRestoreTarget, ReconciledDurableSpillTarget,
};
use quanta_memory_platform_facade::spill::{DurableRecordId, DurableRecordResolution};
#[cfg(feature = "revocable_store")]
use quanta_memory_platform_facade::{
    DurableRestoreOutcome, MemoryPlatformBuilder, RevocableStore, RoutedSpillController,
};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "revocable_store")]
fn expected_payload(index: u64) -> Vec<u8> {
    let mut bytes = vec![0xA5u8; 32];
    bytes[..8].copy_from_slice(&index.to_le_bytes());
    bytes
}

/// Unique scratch database path removed on drop, even on assertion failure.
struct ScratchDb {
    path: PathBuf,
}

impl ScratchDb {
    fn new(tag: &str) -> Self {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "qmp-fup006-{}-{}-{tag}.redb",
            std::process::id(),
            id
        ));
        Self { path }
    }
}

impl Drop for ScratchDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Build and locate the crash-child binary exactly once per test process.
/// A direct `cargo test --test ...` does not rebuild examples, so merely
/// accepting an existing path can execute a stale helper and invalidate the
/// crash boundary. The locked offline build is usually a no-op, but makes
/// source freshness an executable prerequisite.
fn crash_child_binary() -> PathBuf {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY
        .get_or_init(|| {
            let direct = std::env::current_exe()
                .expect("current test binary")
                .parent()
                .expect("deps dir")
                .parent()
                .expect("target dir")
                .join("examples")
                .join(format!("spill_redb_crash_child{}", std::env::consts::EXE_SUFFIX));
            let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .expect("crates dir")
                .parent()
                .expect("workspace root")
                .to_path_buf();
            let workspace_manifest = workspace_root.join("Cargo.toml");
            // Reuse the Cargo binary that launched this test process instead
            // of resolving the rustup shim through PATH in nested test runs.
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let cargo_for_diagnostic = cargo.clone();
            let run_build = |cargo: &OsStr| {
                Command::new(cargo)
                    .arg("build")
                    .arg("--locked")
                    .arg("--offline")
                    .arg("--manifest-path")
                    .arg(&workspace_manifest)
                    .arg("-p")
                    .arg("quanta-memory-platform-facade")
                    .arg("--features")
                    .arg("spill-redb,revocable_store")
                    .arg("--example")
                    .arg("spill_redb_crash_child")
                    .current_dir(&workspace_root)
                    .output()
            };
            let build = match run_build(&cargo) {
                Ok(build) => build,
                Err(first_error)
                    if first_error.kind() == std::io::ErrorKind::NotFound
                        && cargo != OsStr::new("cargo") =>
                {
                    run_build(OsStr::new("cargo")).unwrap_or_else(|fallback_error| {
                        panic!(
                            "freshness build for crash child could not spawn: CARGO error={first_error}; PATH fallback error={fallback_error}; CARGO={cargo_for_diagnostic:?}; PATH={:?}; workspace_root={} (exists={}); manifest={} (exists={}); process_cwd={:?}",
                            std::env::var_os("PATH"),
                            workspace_root.display(),
                            workspace_root.is_dir(),
                            workspace_manifest.display(),
                            workspace_manifest.is_file(),
                            std::env::current_dir()
                        )
                    })
                }
                Err(error) => panic!(
                    "freshness build for crash child could not spawn: {error}; CARGO={cargo_for_diagnostic:?}; PATH={:?}; workspace_root={} (exists={}); manifest={} (exists={}); process_cwd={:?}",
                    std::env::var_os("PATH"),
                    workspace_root.display(),
                    workspace_root.is_dir(),
                    workspace_manifest.display(),
                    workspace_manifest.is_file(),
                    std::env::current_dir()
                ),
            };
            assert!(
                build.status.success(),
                "crash-child freshness build failed: {}",
                String::from_utf8_lossy(&build.stderr)
            );
            assert!(direct.is_file(), "crash-child binary missing at {}", direct.display());
            direct
        })
        .clone()
}

/// Spawn the child, returning its handle plus the lines it prints before the
/// parent kills it. `READY` must arrive within the timeout; anything else
/// (early exit, timeout, unreadable pipe) fails the test with the child
/// stderr attached.
fn spawn_until_ready(db: &Path, mode_args: &[&str]) -> (Child, Vec<String>) {
    let mut child = Command::new(crash_child_binary())
        .arg(db)
        .args(mode_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn crash child");
    let stdout = child.stdout.take().expect("piped stdout");
    let (lines_tx, lines_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut lines = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!remaining.is_zero(), "crash child never printed READY");
        match lines_rx.recv_timeout(remaining) {
            Ok(line) => {
                lines.push(line.clone());
                if line == "READY" {
                    return (child, lines);
                }
            }
            Err(_) => {
                let _ = child.kill();
                let output = child.wait_with_output().expect("reap silent child");
                panic!(
                    "crash child produced no READY (status {}):\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

/// End the child by force and prove it died by the kill — not by exiting on
/// its own (which would downgrade this rail to a clean-shutdown test wearing
/// a crash costume).
fn sigkill_child(child: &mut Child) {
    if let Err(error) = child.kill() {
        // The child died before the kill (a real crash, not a clean exit —
        // no mode exits on its own): reap it and show why instead of a bare
        // kill error.
        let mut stderr = Vec::new();
        if let Some(mut pipe) = child.stderr.take() {
            use std::io::Read;
            let _ = pipe.read_to_end(&mut stderr);
        }
        let status = child.wait().expect("reap dead child");
        panic!(
            "crash child died before SIGKILL (kill error: {error}, status {status}):\n{}",
            String::from_utf8_lossy(&stderr)
        );
    }
    let status = child.wait().expect("reap the crash child");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(9), "child must die by SIGKILL, got {status:?}");
    }
    #[cfg(not(unix))]
    {
        assert!(!status.success(), "child must not exit cleanly, got {status:?}");
    }
}

/// Drain every record oldest-first through the full
/// prepare -> commit -> finalize protocol, returning `(sequence, payload
/// index)` pairs. Any ambiguity fails the test: after a kill, recovery must
/// converge, not strand records behind unknown outcomes.
fn drain_all(backing: &RedbSpillBacking) -> Vec<(u64, u64)> {
    let mut drained = Vec::new();
    while let Some(candidate) = backing.prepare_restore().expect("prepare") {
        let sequence = candidate.sequence;
        let bytes = candidate.bytes().to_vec();
        backing.commit_restore(&candidate.claim).expect("commit converges after kill");
        backing.finalize_restore(&candidate.claim).expect("finalize");
        assert_eq!(bytes.len(), 32, "no torn payload survives a kill");
        let mut index = [0u8; 8];
        index.copy_from_slice(&bytes[..8]);
        assert!(
            bytes[8..].iter().all(|byte| *byte == 0xA5),
            "filler torn by the kill on sequence {sequence}"
        );
        drained.push((sequence, u64::from_le_bytes(index)));
    }
    drained
}

#[test]
fn sep23_fup006_sigkill_after_commit_reopens_with_stable_identity_and_exact_drain() {
    let db = ScratchDb::new("fill");
    let (mut handle, lines) = spawn_until_ready(&db.path, &["fill", "16"]);
    let printed_backend: u128 = lines
        .iter()
        .find_map(|line| line.strip_prefix("BACKEND "))
        .expect("child prints its backend identity")
        .parse()
        .expect("backend identity parses");
    sigkill_child(&mut handle);

    // The survivor reopens the same file cold: the interrupted prepare is
    // claimed under a FRESH generation bound to the SAME persisted backend.
    let backing = RedbSpillBacking::create(&db.path).expect("cold reopen");
    let recovery = backing
        .recover_active_restore()
        .expect("recover")
        .expect("interrupted prepare survives SIGKILL");
    assert_eq!(
        recovery.candidate.claim.backend(),
        printed_backend,
        "backend identity is stable across SIGKILL, not re-minted"
    );
    assert_eq!(recovery.candidate.sequence, 0, "oldest record restores first");
    backing.commit_restore(&recovery.candidate.claim).expect("recovered claim commits");
    backing.finalize_restore(&recovery.candidate.claim).expect("recovered claim finalizes");

    let mut drained = vec![(0u64, 0u64)];
    drained.extend(drain_all(&backing));
    assert_eq!(drained.len(), 16, "every committed record is recovered exactly once");
    for (position, (sequence, index)) in drained.iter().enumerate() {
        assert_eq!(*sequence, position as u64, "oldest-first order survives the kill");
        assert_eq!(*index, position as u64, "payload content survives the kill");
    }
    assert_eq!(backing.retained_count().expect("count"), 0, "exact drain");
    assert_eq!(backing.active_restore().expect("active"), None, "no stranded claim");
}

#[test]
fn sep23_fup006_sigkill_during_write_stream_converges_without_duplicates_or_tears() {
    let db = ScratchDb::new("stream");
    let (mut handle, _) = spawn_until_ready(&db.path, &["stream"]);
    // READY is emitted only after record zero is durable. Killing immediately
    // therefore exercises an arbitrary non-empty committed prefix without a
    // timing assumption. The exact scheduler point relative to the next redb
    // transaction is deliberately not claimed.
    sigkill_child(&mut handle);

    let backing = RedbSpillBacking::create(&db.path).expect("cold reopen");
    let drained = drain_all(&backing);
    assert!(!drained.is_empty(), "the stream committed before the kill");
    let mut previous = None;
    for (sequence, index) in &drained {
        if let Some((previous_sequence, previous_index)) = previous {
            assert!(
                sequence > previous_sequence,
                "sequences strictly increase: {previous_sequence} then {sequence}"
            );
            assert!(
                index > previous_index,
                "payload indexes strictly increase (no dup, no reorder): \
                 {previous_index} then {index}"
            );
        }
        previous = Some((sequence, index));
    }
    // A kill between reserve and commit leaves a genuine hole: low holes with
    // no payload evidence resolve Absent, never Finalized, never guessed.
    let first = drained.first().expect("nonempty").0;
    let last = drained.last().expect("nonempty").0;
    let present: std::collections::BTreeSet<u64> =
        drained.iter().map(|(sequence, _)| *sequence).collect();
    for missing in first..=last {
        if !present.contains(&missing) {
            assert_eq!(
                backing.resolve_record(DurableRecordId::new(missing)).expect("resolve"),
                DurableRecordResolution::Absent,
                "kill-created hole {missing} resolves Absent"
            );
        }
    }
    assert_eq!(backing.retained_count().expect("count"), 0, "exact drain");
    assert_eq!(backing.active_restore().expect("active"), None, "no stranded claim");
}

#[test]
fn sep23_fup006_reserved_hole_stays_absent_across_sigkill() {
    let db = ScratchDb::new("hole");
    let (mut handle, lines) = spawn_until_ready(&db.path, &["hole"]);
    let hole: u64 = lines
        .iter()
        .find_map(|line| line.strip_prefix("HOLE "))
        .expect("child prints its reserved hole")
        .parse()
        .expect("hole id parses");
    sigkill_child(&mut handle);

    let backing = RedbSpillBacking::create(&db.path).expect("cold reopen");
    assert_eq!(
        backing.resolve_record(DurableRecordId::new(hole)).expect("resolve"),
        DurableRecordResolution::Absent,
        "a reserved-never-committed id is Absent after SIGKILL, not Finalized"
    );
    let drained = drain_all(&backing);
    assert_eq!(drained, vec![(0, 0), (1, 1)], "only committed records drain");
    assert_eq!(backing.retained_count().expect("count"), 0, "exact drain");
}

#[cfg(feature = "revocable_store")]
#[test]
fn durable_spill_store_charge_and_record_survive_process_boundary() {
    let db = ScratchDb::new("store-spilled");
    let (mut child, lines) = spawn_until_ready(&db.path, &["store-spilled"]);
    assert!(lines.iter().any(|line| line == "CHARGE 0"));
    sigkill_child(&mut child);

    let platform = MemoryPlatformBuilder::new()
        .with_preset(MemoryPreset::Development)
        .build()
        .expect("survivor platform");
    let store = platform
        .revocable_store_builder::<String, Vec<u8>>()
        .bind(StoreBindingHandle::new(
            MemoryClass::ArtifactCache,
            "durability-spill-survivor".to_string(),
            None,
        ))
        .build()
        .expect("survivor store");
    let controller =
        RoutedSpillController::new(RedbSpillBacking::create(&db.path).expect("cold reopen"));
    assert!(matches!(
        store.restore_one_durable_bounded(&controller).expect("restore committed record"),
        DurableRestoreOutcome::Restored { sequence: 0 }
    ));
    assert!(platform.broker().ledger_snapshot().expect("resident ledger").total_owner_bytes() > 0);
    assert_eq!(
        store.reclaim_one().expect("read restored"),
        Some(("k0".to_string(), expected_payload(0)))
    );
    assert!(matches!(
        store.restore_one_durable_bounded(&controller).expect("second restore"),
        DurableRestoreOutcome::Empty
    ));
    assert_eq!(controller.spilled_count(), Ok(0));
    assert_eq!(platform.broker().ledger_snapshot().expect("drained").total_owner_bytes(), 0);
}
