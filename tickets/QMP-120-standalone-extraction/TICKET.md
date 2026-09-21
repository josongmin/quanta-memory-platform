# QMP-120 — Deterministic standalone extraction and release rail

Priority: P1  
Lifecycle authority: `tickets/README.md`  
Depends on: QMP-000  
Blocks: QMP-130, QMP-140

Status: `IMPLEMENTED`. The local isolated source at `4fdac520` freezes the exact
committed HEAD/tree, derives the copy manifest from Git tree objects, binds each
path to mode/blob/content identity, rejects tracked/index/mode drift, and passed
a fresh actual extraction with clean destination commit. See
[`receipts/2026-09-21T055538Z-4fdac52.md`](receipts/2026-09-21T055538Z-4fdac52.md).
The repository does not use GitHub Actions as extraction authority. A fresh
repo-local actual extraction of the workflow-free canonical set is required
before promotion to `QUALIFIED`.

## Objective

dry-run이 아니라 fresh destination actual extraction을 재현 가능하게 통과시키고, extracted repository가 필요한 tool/config/lockfile을 모두 자체 보유하며 clean 상태가 되게 한다.

## Confirmed RED receipt

```text
scripts/extract-to-standalone-repo.sh /tmp/qmp-boundary-audit.h8nz5W
exit 7
```

Historical pre-fix RED recheck (product `e047215a5c98c2f57f4f53281da4d4c055e82f92`,
clean detached worktree, 2026-09-21): actual extraction to fresh
`/tmp/qmp-120-red-qgMeWA` exits 7 at `regression-guards` after creating
bootstrap commit `1e6fc5a` **before verification**. Extracted tree contains
untracked, newly resolved `Cargo.lock`, and lacks
`tools/semgrep/qmp-regressions.yml`. This is a RED reproduction, not an
extraction success receipt. QMP-000 was QUALIFIED; this RED is not a claim that
the currently integrated implementation remains `READY` or still fails.

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
- extraction tests and repo-local qualification scripts

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
12. clean/frozen source에서 temp destination을 사용한 actual extraction을
    qualification front door로 실행한다.

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
- [x] extraction input가 declared `HEAD` tree/mode와 동일함을 검증한다.
- [x] dirty tracked source와 executable-bit drift negative fixtures가 RED다.

Current-source refresh: actual extraction of
`f1d45c9bb28982f2c5ee3fcaac4a5d8ce4329be9` passed the complete 638-test
doctor, feature-runtime lanes, selector validator, PM lint, deny and clean Git
postcondition. See
[`receipts/2026-09-21T105625Z-f1d45c9.md`](receipts/2026-09-21T105625Z-f1d45c9.md).

Current-source session-hardening refresh: actual extraction of
`557213b01ed93a20c1a45d3c69b2dae20ab90e70` produced canonical digest
`da315e42ec3d2c8a028119db9c26082130f763bd2256c269f099012dcb0fc8c3`,
bootstrap commit `6f744cac4e36d9dae5439c832d5a0a41f7809199`, and a clean extracted
worktree after the complete 639-test doctor, ticket validator and PM lint.
See
[`receipts/2026-09-21T131415Z-557213b.md`](receipts/2026-09-21T131415Z-557213b.md).

Policy correction (2026-09-22): `.github/workflows/standalone-extraction.yml`
was not an operational rail and is removed from the repository and canonical
copy set. Historical receipts that recorded it as unobserved remain immutable
history; they are not a current acceptance requirement. QMP-120 closes from an
exact-source repo-local actual extraction receipt.

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

## Final-audit P1-F03 — resolved locally at `4fdac520`

The prior extractor built its file list with `git ls-files` but copied bytes from
the working tree. It neither rejected tracked dirtiness nor compared copied
blobs and modes with `HEAD`. The digest covered path/content but not executable
mode, so a receipt could name `HEAD=X` while extracting different bytes.

Implemented repair:

1. Freeze source `HEAD` and tree ID; reject relevant tracked/index dirtiness at
   both start and end, or extract directly from the frozen tree object.
2. Derive the manifest from `git ls-tree -r HEAD` and bind mode, blob ID, path,
   content digest, and destination mode.
3. Emit the frozen tree ID and canonical manifest digest in the receipt.
4. Add dirty-tracked, staged-only, executable-bit and repeated-extraction
   fixtures; retain the existing omitted-source and pre-commit failure contracts.
5. Two extractions from the same tree must produce the same canonical manifest
   digest; a different tree may never reuse the prior receipt.

Local checkpoint:

```text
source HEAD: 4fdac520e877e56a1c3a51dcd8ee715d1f73d18b
source tree: af7d81b6d1b04360680584ec5fb0cf2870f76277
canonical manifest SHA-256: 53bc8aebf3cb77fd31d71aa157538c7c2b0a50a54c47265bbfc81165a3ef1847
extracted bootstrap commit: 9eadc03104c894fbc21a34414eba48abe66af22b
extracted git status: clean
```
