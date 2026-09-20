# Frozen audit baseline

이 문서는 `tickets/README.md`가 관리하는 remediation campaign의 입력 evidence다. 상태나 완료 여부를 표현하지 않는다.

## Snapshot

```text
repository: /Users/songmin/Documents/code-new/quanta-memory-platform
branch: main
HEAD: e103771670da3143e2ffebb8950a5dfc7e3d4515
audit date: 2026-09-21
product source dirty: none
auditor-owned untracked:
  AUDIT-2026-09-21.md
  SYSTEM-AUDIT-CHECKLIST.md
```

External host static snapshot:

```text
repository: /Users/songmin/Documents/code-new/semantica-codegraph-v2
HEAD: e9168a73b4abca24fd177a726de150696d40c3eb
state: shared dirty main; many unrelated modified/untracked paths
proof level: STATIC only
```

## Purpose verdict

`quanta-memory-platform`의 stated purpose는 portable, host-independent memory budgeting/accounting authority와 standalone extraction이다.

현재 verdict는 `FAIL`이다.

- dependency direction과 pure-layer fence는 유지된다.
- covered ledger/store mutation은 상당한 invariant test를 가진다.
- 그러나 public/default/runtime/lifecycle/spill 경로에 accounting bypass 또는 leak가 있다.
- actual extraction이 실패한다.

## Confirmed blocking behavior

1. mandatory class 누락 시 ledger class cap이 적용되지 않는다.
2. live broker runtime scratch는 basic uncharged allocator다.
3. default builder는 report-only broker다.
4. raw ledger/basic broker authority가 public이다.
5. charged store drop은 live ticket을 환급하지 않는다.
6. four store types는 populated state에 late accounting sink attach가 가능하다.
7. key/identity/index/tombstone cost가 charge 대상에서 빠진다.
8. transcript process-lifetime tombstone이 무제한이다.
9. reclaim 후 in-memory spill은 resident byte를 uncharged 상태로 옮긴다.
10. standalone extraction copy plan이 실행 의존성과 lockfile을 누락한다.

상세 근거는 루트 `AUDIT-2026-09-21.md`의 P1/P2 section을 사용하되, 구현 범위와 완료 조건은 각 ticket이 우선한다.

## Positive evidence

- resolved dependency direction: `core <- facade <- testkit`
- domain/ports purity scan: prohibited direct I/O/clock/adapter/host import 미발견
- `cargo test --workspace --all-features`: PASS, 404 total
- `scripts/memory-platform-doctor`: PASS
- `scripts/check-workspace.sh`: PASS
- `python3 tools/pm/pm.py lint`: PASS
- `cargo deny --all-features check`: PASS
- extraction dry-run: PASS

이 GREEN들은 아래 actual extraction failure와 missing negative paths를 닫지 않는다.

## Negative receipt

```text
command:
  scripts/extract-to-standalone-repo.sh /tmp/qmp-boundary-audit.h8nz5W
exit:
  7
failure boundary:
  extracted scripts/regression-guards.sh invokes missing
  tools/semgrep/qmp-regressions.yml when semgrep is installed
extracted status:
  ?? Cargo.lock
observed dependency drift:
  moka 0.12.15 -> 0.12.16
  scc 3.8.7 -> 3.8.8
  hdrhistogram 7.5.4 -> 7.6.0
```

## Proof gaps at baseline

- no Criterion/IAI/Loom/fuzz targets
- no production workload RSS/performance threshold
- conformance oracle is not independent and maps snapshot error to zero
- metrics adapters have no live broker/store emission call sites
- external host behavior tests were not run on its dirty shared main
- shipped examples do not prove enforcement

