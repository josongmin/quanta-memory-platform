# Sep-23 follow-up misc — historical execution record

Status: **HISTORICAL**. Redb/durable FUP-001/002 and related release gates are
excluded from the current product plan. See the
[active in-memory scope](../sep-24-adversarial-e2e-bench/README.md). Prior
implementation and receipts remain historical evidence, not current release work.
Current status is summarized in the [plan index](../README.md): FUP-003/004/005/007
are implemented and owner-verified; FUP-006's last local/host pair receipt
names QMP `7f1c2f8`. The clean `db0b03c` audit base has a later benchmark
verifier change, so it needs a new qualification receipt. Historical text
below describes earlier candidate states and does not override that boundary.
작성일: `2026-09-23`
계획 기준 HEAD: `028448946caa95676df034481a830c1d3e3b1f33`
계획 기준 tree: `f983669731805a330200213bad7bf1bd5573f308`

The sequence and writer lanes below record the 2026-09-23 campaign. Do not
dispatch its Redb lane or carry its durable qualification into the current
release checklist. The active scope is core accounting, in-memory stores,
reclaim, and in-process transfer; existing optional Redb code is unchanged.

## 목적과 증거 범위

현 HEAD 감사에서 확인한 **P1 독립 근본 원인 3건**(durable record 상태,
durable recovery 단일 소유권, 0-byte ticket의 scope 수명)과 P2 개선 3건을
구현 가능한 티켓으로 분리했다. P0으로 확정된 건은 없다. FUP-001~005/007은
현재 QMP 소스에서 구현·focused GREEN·temporary-mutant RED가 확보됐다.
QMP repository/standalone extraction은 predecessor tag
`qmp-final-qualified-sep23-followup`에서 GREEN이었다. 그 뒤 발견된 scheduler
deny-path allocation 회귀를 lazy `GroupAdmission`으로 수정한 후, durable claim-bearing
public value도 non-`Clone`/non-`Copy`로 만들어 하나의 backend-issued lease가
복수 caller-owned restore flow로 분할되지 않게 했다. v3/v4는 기존 source commit에
대한 불변 attestation으로 보존한다. 신규 boundary oracle 둘을 포함한 현재 후보에서
canonical repository, real extraction, full scaling verifier가 통과했다. 이후 dirty한
활성 sibling checkout을 빌드 입력으로 쓰지 않고 clean detached bundle을 만들어
exact-source host-pair도 통과했다. 현재 full-followup 후보의 local qualification은
완료했으며 v5 tag/push는 별도 승인 경계라 발행하지 않았다. 이는 별도 100항목 제품
체크리스트의 완료가 아니며 B07/J06 및 물리/운영 경계는 OPEN이다. 위의
extraction/benchmark/host-pair 결과는 후속 Redb schema/resource 변경 이전의
후보에만 적용된다. 현재 후보는 별도로 동결·재검증해야 한다. 정확한
source/result tuple과 잔여 경계는
`RECEIPT.md` 및 `tickets/HOST-PAIR-RUNTIME-DEFAULT-PROVENANCE.md`가 소유한다.

- **실행 재현:** 실제 Redb의 동시 recovery가 같은 sequence를 두 store에 복원;
  예약만 된 ID가 후속 ID의 finalize 뒤 `Finalized`로 판정됨.
- **실행 재현:** 0-byte grouped `ManagedCharge`를 보유한 채 group/owner mapping을
  제거하는 live reconfigure가 성공한 뒤 release와 deferred drain이 실패한다.
  pending permit Drop의 권한 유실은 같은 root의 소스 경로이며 별도 실행은 아직 없다.
- **실행 재현:** public broker에서 불가능한 요청이 기존 charge를 회수한 뒤 거절;
  같은 group 사용량에서도 soft crossing 여부에 따라 최초 reclaim ladder가 달라짐.
- **정적 확인:** kind 교차검증 공백, accepted reconfigure마다 provenance 누적.
  정상 API로 도달 가능한 kind corruption이나 provenance로 인한 운영 장애는 미입증.
- 일부 최초 재현은 `943cf0a`와 이후 review-correction overlay에서 수행했다.
  현 HEAD `0284489`의 durable 두 건은 격리 clone에서 각각 1개 선택된 계약 테스트가
  기대값 차이로 실패했다. 0-byte committed 경로와 public reclaimer 두 경로도 격리
  실행했다. 정확한 명령·출력·미실행 범위는
  `docs/misc/memory-state-substrate-gap-reaudit-2026-09-23.md`와 최초 감사 보고서에 있다.
  이 티켓의 `sep23_fupNNN` selector는 현재 owner tests로 구현되어 non-zero
  selection과 GREEN을 확인했다.
- 기존 `qmp-final-qualified-d370529`는 현재 후보의 qualification이 아니다.
  기존 late-attach compile-fail 교정과 ticket index의 `BTreeMap` 복원은 이번 작업에서 보존한다.

## 당시 티켓과 실행 순서 (현재 dispatch 금지)

| 티켓 | 우선순위 / 성격 | 담당 lane | 선행 조건 |
| --- | --- | --- | --- |
| [FUP-001 — 정확한 durable record 상태](FUP-001-durable-record-state.md) | historical / current plan excluded | A | source freeze |
| [FUP-002 — recovery 단일 소유권](FUP-002-recovery-ownership.md) | historical / current plan excluded | A | FUP-001 |
| [FUP-007 — 0-byte ticket scope 수명](FUP-007-zero-byte-ticket-scope.md) | P1 / correctness | B0 | source freeze; `control.rs` 단독 lease |
| [FUP-003 — admission preflight와 reclaim](FUP-003-admission-reclaim.md) | P2 / 정책 품질 | B | FUP-007, FUP-004, FUP-005 |
| [FUP-004 — kind 교차검증](FUP-004-kind-axis-validation.md) | P2 / 방어 강화 | C | FUP-007의 ledger lease 반환 |
| [FUP-005 — bounded provenance](FUP-005-bounded-provenance.md) | P2 / 운영 비용 | B | FUP-007의 control lease 반환 |
| [FUP-006 — 통합·exact-source qualification](FUP-006-integration-qualification.md) | in-memory release gate; durable branch historical | I | FUP-007; P2는 후보 포함 시 추가 |

1. **P1 Wave 1:** A=001과 B0=007을 병렬 진행한다. A는 facade의 Redb/spill/revocable,
   B0는 core의 `api/control.rs`와 필요한 ledger/scheduler 검증 파일을 독점한다.
2. **P1 Wave 2:** A=002는 001의 persisted schema/호환 계약을 인수한 뒤 같은 파일
   lease에서 이어서 한다. B0는 007의 ledger/control lease를 반환한다.
3. **P2 Wave:** C=004는 007이 `ledger.rs`를 반환한 뒤, B=005는 007이 `control.rs`를
   반환한 뒤 시작할 수 있다. B=003은 004/005 결과와 두 파일 lease를 인수한 뒤 시작한다.
4. **통합:** full-followup 범위로 구현됐다. 최종 고정 source의 canonical/extraction
   결과와 host-pair GREEN을 `RECEIPT.md`와 annotated tag에 분리 기록한다.

당시 P1이 우선이었다. `spill-redb`는 opt-in이므로 **사용하지 않는다는 host/dependency graph
증거가 있을 때만** durable 미해결 상태를 기능 비활성 범위로 분리할 수 있다. 이 경우에도
007은 core 경로이므로 남는다. 사용 여부가 미확인이면 durable 기능을 PASS로 추정하지 않는다.

## 파일 소유권과 충돌 방지

- A: facade의 `spill/{mod,service}.rs`, `spill/backings/redb.rs`,
  `revocable_store/service.rs`, 필요한 facade public API 테스트/re-export.
- B0: FUP-007 동안 core `api/control.rs`, 필요 시 `domain/ledger.rs`,
  `application/scheduler.rs`, owner-local policy/public API 테스트. 같은 파일을 쓰는 B/C는
  B0의 handoff 전에는 쓰지 않는다.
- B: core의 `api/control.rs`, report/provenance와 admission 관련 domain/port/API 파일.
  `ledger.rs`는 **007→004 완료 후 003에서만** 편집한다.
- C: core `domain/ledger.rs`와 testkit accounting view/model/fake/contract.
  `api/control.rs`와 report/provenance 파일은 편집하지 않는다.
- I: 루트 README/설계 가이드/MIGRATION/EXTRACTION/검증 문서, 공통 예제의 통합,
  이 디렉터리의 상태 집계와 receipt. 각 lane은 공통 문서 변경 요구를 handoff에 적는다.
- 같은 파일의 테스트와 production 영역도 하나의 lease로 취급한다. 파일 내부가
  다르다는 이유로 두 writer를 동시에 투입하지 않는다. 공개 export나 공통 fixture의
  추가 write set은 다른 owner와 합의한 뒤만 편집한다.

## 공통 실행 규칙

1. 시작 시 HEAD/tree, dirty 목록, owner 파일 hash, 실제 dependency 경로를 기록한다.
   사용자 변경을 reset/stash/삭제하지 않는다. 외부 변경을 자신의 변경으로 집계하지 않는다.
2. 현재 증상을 owner-local 테스트로 재현하고, 올바른 동작을 기대하는 assertion의 RED를
   먼저 확보한다. 잘못된 동작을 기대하는 감사용 probe의 PASS는 제품 PASS가 아니다.
3. 수정 후 같은 테스트 GREEN과 좁은 regression suite를 실행한다. 임시 mutant는
   공유 checkout이 아닌 별도 snapshot에서 실행하고, 원복 hash를 확인한다.
4. 티켓의 `sep23_fupNNN` selector는 **신규 테스트의 예정 prefix**다. 구현 후 목록에서
   실제 존재와 선택 건수를 확인한다. `0 passed / 0 selected`는 증거가 아니다.
5. 원장 fail-closed, single-use authority, lock 밖 callback 경계, builder-first API,
   `core <- facade <- testkit` 방향을 유지한다. domain/ports에 I/O나 실제 시계를 넣지 않는다.
6. 기존 테스트를 의미 없이 재작성하거나, assertions/feature gate를 약화하거나,
   source 문자열 검사로 semantic oracle를 대체하지 않는다.
7. writer는 자신의 focused rail만 실행한다. 전체 workspace/feature/extraction 검증은
   I가 공통 source에서 실행하여 중복을 줄인다. API/feature 변경에 필요한 좁은 체크는 생략하지 않는다.

## 공통 handoff / receipt 형식

각 티켓 하단에 실제 수행 후 다음 항목을 채운다. 계획만으로 DONE으로 바꾸지 않는다.

```text
Status: OPEN | IN_PROGRESS | IMPLEMENTED | VERIFIED
Preimage: HEAD/tree, dirty ownership, owner hashes
Postimage: HEAD/tree 또는 candidate digest, owner hashes
Changed: 파일별 동작 변경과 API/저장 포맷 영향
Proof: 정확한 command, exit code, selected count, RED -> GREEN, artifact 경로
Compatibility: 기존 caller / feature profile / 기존 DB 처리
Not run: 미실행 범위와 이유
Residual risk: 남은 제약 / 운영 경계
Handoff: lease 반환 파일, 다음 lane 시작 조건
```

`VERIFIED`는 ticket-local proof다. repository/extraction/host/deployment/activation은
006에서 별도 상태로 기록한다. 외부 source freeze 요청 중에는 문서도 편집하지 않는다.
이 계획은 100점 체크리스트 전체 closure가 아니다. 007은 B07의 ticket metadata
**개수 상한**까지 닫지 않으며, 001/002도 disk/codec budget·power-loss model을
자동으로 닫지 않는다. 그 범위는 별도 적용성/증거 판정으로 남긴다.

## 이번 wave에서 하지 않을 것

- Hamilton을 dynamic borrowing으로 기본 교체, scheduler 전면 개명.
- sharding/lock-free, DRF, S3-FIFO, hysteresis의 근거 없는 기본 도입.
- 이미 있는 policy revision/atomic reconfigure를 새 subsystem으로 중복 구현.
- physical telemetry로 semantic ledger를 보정하거나 host OS 접근을 core에 추가.
- 기존 전체 audit checklist를 전부 신규 blocker로 전환.

이 항목들은 workload/lock/pressure 측정이나 별도 제품 계약이 생기면 새 티켓으로 분리한다.
