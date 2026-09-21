# QMP-140 — Public API SemVer, documentation truth, and terminal closeout

Priority: P2  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-010, QMP-020, QMP-030, QMP-040, QMP-050, QMP-060, QMP-070, QMP-080, QMP-090, QMP-100, QMP-110, QMP-120, QMP-130  
Blocks: campaign DONE

## Objective

remediation으로 바뀐 public authority를 compatibility 정책에 맞게 노출하고, README/EXTRACTION/verification 문서가 실제 receipt보다 앞서가지 않게 하며 최종 campaign closure를 수행한다.

## Baseline evidence requiring final-source reconciliation

- 최초 감사에서 raw ledger/basic broker/simple store constructors가 public tests에 pin되고 builder-first 주장과 public backdoors가 모순됐다. QMP-030/040 이후 API가 이동했으므로 이를 **현재의 미수정 결함으로 재주장하지 않는다**. 최종 re-export/feature census와 외부 compile-fail로 남은 노출을 판정한다.
- `docs/verification-scenarios.md`는 benchmark가 없는데 benchmark 비율/gate를 주장한다.
- 최초 감사 시 actual extraction failure와 standalone green/no known limitation 주장이 충돌했다. QMP-120은 local extraction을 수리했으나 remote CI와 최종 product SHA의 extraction은 아직 별도 증거가 필요하다.
- moka/scc/bumpalo feature 이름이 실제 semantic capability보다 강하게 보인다.
- packages는 `rust-version` 미지정, `UNLICENSED`, `publish=false`다.
- QMP-060 removes the misleading unbounded `TranscriptRetentionPolicyV1`
  surface in favor of bounded `TranscriptRetentionPolicyV2`, adds typed
  saturation/reconfigure/stats contracts, and adds a required trait method.
  This is a deliberate pre-1.0 breaking change that needs final migration and
  semver reporting in this ticket; do not reintroduce a compatibility alias
  that restores unbounded behavior.

## Canonical write set

- crate `lib.rs` re-exports
- public API tests and doctests
- facade/core `Cargo.toml` features/package metadata
- `README.md`
- `EXTRACTION.md`
- `docs/verification-scenarios.md`
- release/changelog/migration documentation
- `tickets/README.md` final states and receipts

Generated agent guidance를 수정해야 할 경우 `tools/pm/sources/**`만 수정하고 `pm.py sync`를 실행한다.

## Required decisions

1. SemVer versioning strategy:
   - pre-1.0 breaking release 또는 staged deprecation
   - compatibility window
   - feature quarantine policy
2. public raw authority:
   - remove/private
   - explicit `advanced-unchecked` feature
   - safety/guarantee loss 문서
3. unbudgeted types의 naming/feature visibility.
4. backing feature naming과 reachable capability 일치.
5. MSRV (`rust-version`)와 toolchain policy.
6. internal-only vs distributable license/publish policy.

## Required documentation outcome

- README happy path는 enforced builder와 cap denial/final drain을 보여준다.
- report-only/unbudgeted path는 별도 warning과 type name을 가진다.
- 정책 field semantics가 implementation과 일치한다.
- extraction 문서는 actual locked extraction 명령과 receipt를 제시한다.
- verification scenarios는 executable test/bench 이름과 정확히 매핑된다.
- known limitations가 0이라고 쓰려면 open ticket/proof gap이 실제 0이어야 한다.
- external host boundary와 별도 qualification requirement를 명시한다.

## Public API tests

- removed raw constructors are compile-fail externally
- enforced builder path compiles with documented example
- explicit unbudgeted/report-only path naming is pinned
- feature matrix exposes only reachable capabilities
- deprecated compatibility path emits expected warning and cannot bypass accounting
- cargo-semver-checks 또는 equivalent report를 capture

## Terminal audit

1. re-freeze QMP HEAD and dirty ownership.
2. rerun every ticket validator/crosswalk.
3. run workspace rails.
4. run actual extraction from fresh temp directory.
5. verify extracted git clean and source digests.
6. attach QMP-130 host exact-head receipt.
7. attach QMP-100 PERF receipt.
8. inspect docs for stale claims and nonexistent selectors.
9. set ticket states only from receipts.

## Acceptance

- [ ] no public API claim contradicts visibility/runtime behavior.
- [ ] all breaking changes have migration notes and version decision.
- [ ] every feature name maps to reachable tested capability.
- [ ] README example proves enforcement, not report generation only.
- [ ] docs contain no nonexistent benchmark/gate/test claim.
- [ ] workspace, extraction, host, performance receipts share declared exact revisions.
- [ ] all P1/P2 crosswalk entries are closed.
- [ ] final `tickets/README.md` lifecycle is internally consistent.
- [ ] every documented named test selects at least one test; zero-test GREEN is rejected.
- [ ] ticket/receipt schema enforces state, claim, identity, SHA and selected-count truth.

## Verification

```bash
python3 tools/pm/pm.py sync
python3 tools/pm/pm.py lint
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo deny --all-features check
scripts/check-workspace.sh
scripts/memory-platform-doctor
scripts/verify-tickets.sh
```

그리고 QMP-120 actual extraction, QMP-130 host, QMP-100 perf receipts가 필수다.

## Non-goals

- 새로운 product feature 추가
- receipt 없이 “SOTA”, “production-ready”, “no known limitation” 선언

## Final-audit documentation/proof drift

- `docs/verification-scenarios.md` still describes saturating ledger totals and
  names a removed saturation test, while current code uses checked aggregation
  and a different overflow-denial oracle.
- `EXTRACTION.md` names host test targets absent from the current Semantica tree.
- The feature matrix compiles and clippy-checks leaf features but does not execute runtime
  capability tests; this cannot qualify advertised moka/scc backings.
- `README.md` describes historical extraction failure at the audit base but does
  not clearly distinguish the later local extraction passes from unobserved
  remote CI and the new HEAD-binding gap.

Add a docs/test-selector linter that resolves every command against `cargo test
-- --list` or the host suite registry, a runtime bite test for every advertised
leaf feature, and machine-readable receipt validation from QMP-000. Remove or
quarantine features whose canonical builder path is not reachable.
