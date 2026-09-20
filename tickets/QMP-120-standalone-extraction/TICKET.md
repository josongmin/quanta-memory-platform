# QMP-120 — Deterministic standalone extraction and release rail

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-000  
Blocks: QMP-130, QMP-140

Status: `IMPLEMENTED` at product `e6f69c340239784a2d5af7d685ba8894c47ee0c1`.
Local exact-content extraction and offline checks passed; the new CI workflow
has not yet produced a remote run. See
[`receipts/2026-09-20T233523Z-e6f69c3.md`](receipts/2026-09-20T233523Z-e6f69c3.md).

## Objective

dry-run이 아니라 fresh destination actual extraction을 재현 가능하게 통과시키고, extracted repository가 필요한 tool/config/lockfile을 모두 자체 보유하며 clean 상태가 되게 한다.

## Confirmed RED receipt

```text
scripts/extract-to-standalone-repo.sh /tmp/qmp-boundary-audit.h8nz5W
exit 7
```

Current-source recheck (product `e047215a5c98c2f57f4f53281da4d4c055e82f92`,
clean detached worktree, 2026-09-21): actual extraction to fresh
`/tmp/qmp-120-red-qgMeWA` exits 7 at `regression-guards` after creating
bootstrap commit `1e6fc5a` **before verification**. Extracted tree contains
untracked, newly resolved `Cargo.lock`, and lacks
`tools/semgrep/qmp-regressions.yml`. This is a RED reproduction, not an
extraction success receipt. QMP-000 is QUALIFIED, so this ticket is `READY`,
not dependency-blocked.

원인:

- copy plan이 `tools/semgrep/qmp-regressions.yml`을 누락.
- extracted `scripts/regression-guards.sh`는 semgrep 설치 시 해당 파일을 요구.
- `Cargo.lock` 누락으로 dependency re-resolution과 dirty worktree 발생.
- observed drift: moka, scc, hdrhistogram patch versions.
- extraction script는 verification 전에 bootstrap commit을 생성.

## Canonical write set

- `scripts/extract-to-standalone-repo.sh`
- `scripts/lib/self-hosting-manifest.sh`
- `scripts/{check-workspace,memory-platform-doctor,regression-guards}.sh`
- `tools/semgrep/**`
- `tools/ci/feature-matrix.sh`
- root `Cargo.lock`, `.cargo/**`, toolchain/MSRV config as decided
- extraction tests/CI

## Required implementation

1. copy plan을 executable dependency closure에서 생성/검증한다.
2. `tools/semgrep/qmp-regressions.yml`을 required path에 포함한다.
3. `Cargo.lock` 정책을 결정하고 standalone application/library release 목적에 맞게 포함한다.
4. all Cargo rails에 `--locked`; offline lane은 별도 `--frozen`으로 표시한다.
5. verification을 bootstrap commit 전에 실행한다.
6. verification 실패 시 destination이 “성공한 repo”처럼 commit된 상태로 남지 않는다.
7. source/extracted canonical file digest를 비교한다.
8. extracted repo에서 forbidden monorepo dependency/path를 검사한다.
9. final `git status --porcelain`이 empty인지 검사한다.
10. cargo-deny/tool absence를 release lane에서 skip하지 않는다.
11. feature matrix는 each feature alone + `-D warnings`를 적용한다.
12. temp destination을 사용한 actual extraction을 CI에서 실행한다.

## RED tests/scenarios

- semgrep installed / unavailable 두 환경
- missing semgrep config fixture
- missing lockfile fixture
- lockfile drift with `--locked`
- forbidden host path dependency
- source file omitted from copy manifest
- extracted verification failure before commit
- final dirty file detection
- repeated extraction produces identical canonical digest

## Acceptance

- [x] actual extraction이 fresh temp directory에서 exit 0.
- [x] extracted repo의 check-workspace와 doctor가 모두 PASS.
- [x] online Cargo rails use `--locked`; separate offline rail uses `--frozen`.
- [x] extracted git status가 clean.
- [x] source/extracted digest match.
- [x] no Semantica host noun/path dependency introduced.
- [x] failure path가 invalid success commit을 만들지 않는다.
- [x] release receipt에 rustc/cargo/cargo-deny/semgrep versions가 기록된다.

Remote `.github/workflows/standalone-extraction.yml` execution remains
unobserved; keep `IMPLEMENTED`, not `QUALIFIED` or `DONE`.

## Verification

```bash
qmp_extract_dir="$(mktemp -d /tmp/qmp-extract.XXXXXX)"
scripts/extract-to-standalone-repo.sh "${qmp_extract_dir}"
git -C "${qmp_extract_dir}" status --porcelain
cargo metadata --locked --format-version 1 --manifest-path "${qmp_extract_dir}/Cargo.toml"
```

환경 변수는 explicit validated temp path만 사용한다. broad delete나 source workspace mutation은 금지한다.

## Non-goals

- Semantica rewire: QMP-130
- crates.io publish decision: QMP-140
