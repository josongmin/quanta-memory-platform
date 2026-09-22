# FUP-003 — 공유 admission preflight와 단일 reclaim 계획

Status: `OPEN`
Priority: `P2`
Owner: `Lane B`
Depends on: `FUP-007`의 `control.rs`/`ledger.rs` lease 반환, `FUP-004` ledger lease 반환, `FUP-005` control/report 변경 반영
Blocks: `FUP-006`의 full-followup 후보. P1-only 후보에서는 OPEN으로 남을 수 있다.
Checklist: `F07`의 불필요한/부정확한 reclaim 판단; `C01`, `J01`, `K06`의 decision/receipt 일부. `F06` callback 재진입과 일반 callback 시간 상한은 이 티켓만으로 닫지 않는다.

## 문제와 근거

`api/control.rs::reserve_with_reclaim`은 실제 admission보다 먼저 soft callback을 실행한다.
class 사용량 40 / soft 50 / hard 100에서 신규 101 요청은 기존 40을 회수한 뒤 거절된다.
group 사용량 40 / soft 45 / hard 50 / 신규 20 / global 1000에서는 Soft 계획에
cross-class rung이 없어 거절되지만, soft 30인 대조군은 한 번의 Hard 계획으로 승인된다.

이는 회계 corruption이 아니라 불필요한 회수와 최초 계획 선택 문제다.
`max_attempts=1` 계약은 유지한다. callback 이후 현재 ledger로 재측정하므로
policy revision 누락을 cap 우회로 주장하지 않는다. in-repo production reclaimer는
없으며, 감사 재현은 실제 broker와 공개 port를 따르는 테스트 reclaimer를 사용했다.

현 HEAD 공개 API 격리 probe의 출력은 `impossible_denied=true callbacks=1
remaining_bytes=0`이다. 요청 101 자체가 class hard 100을 넘는데 기존 40-byte
charge가 회수됐다. group 비교 probe는 soft 45에서 `admitted=false`, Soft 계획
1회였고, soft 30에서 Hard 계획의 `LowerPriorityClasses` rung이 추가되어
`admitted=true`였다. 명령/코드는
`docs/misc/memory-state-substrate-gap-reaudit-2026-09-23.md`에 기록돼 있다.
둘 다 설치된 callback이 계획을 따르는 조건부 동작이며 현 host의 reclaimer 설치는
확인되지 않았다. 운영 P1/데이터 손실로 격상하지 않는다.

## Write set

- core `src/api/{control,broker}.rs`, `src/domain/ledger.rs`의 admission owner 부분.
- core `src/ports/reclaim.rs`, 필요한 `src/application/scheduler.rs`의 순수 admission 결과.
- 새 typed decision의 기본 위치는 core `src/domain/admission.rs`; `domain/mod.rs`,
  필요 시 `domain/errors.rs`와 API module wiring을 함께 갱신한다.
- core `src/lib.rs`, `tests/{policy_contracts,public_api_surface}.rs`의 직접 API 영향.
- testkit 공통 파일은 C의 결과를 읽기만 한다. 추가 변경이 필요하면 별도 lease를 받는다.

## 구현 단계

1. 감사의 3개 시나리오를 올바른 동작을 기대하는 owner-local 테스트로 옮긴다.
   impossible 요청은 callback 0회와 기존 charge 보존, group denial은 적절한 최초
   계획 1회와 최종 admission을 검사한다. counter rollback을 callback에 요구하지 않는다.
2. `ledger::validate_reserve_postimage`, `reserve`, `control::reserve_on`, scheduler의
   현재 조건을 목록화하고 **mutation과 preflight가 같은 순수 predicate를 사용**하도록
   추출한다. 복사한 별도 cap 계산기나 문자열 error parsing을 만들지 않는다.
3. 내부 결과를 `Admit / ReclaimCandidate / TerminalDenial / BusyOrInvalid`로 분리한다.
   request 자체가 유효한 burst/floor/headroom까지 고려한 최대 허용량을 초과하는 등
   **불가능함을 증명한 경우만** Terminal로 분류한다. ReclaimCandidate는 실제 victim
   존재/회수 성공 보장이 아니라 회수를 시도할 수 있다는 뜻이다.
4. global/class/owner/group/shared-pool/protected-floor/execution-scratch와 산술 overflow,
   미등록 class/group, rebuild, ticket exhaustion을 다룬다. 요청의 context/kind도 전달한다.
   class/group의 hard cap을 borrowing 허용 여부와 무관한 절대 상한으로 해석하지 않는다.
5. locked preflight가 Terminal/Busy면 callback 없이 반환한다. ReclaimCandidate면 실제
   제한 축에 맞는 최초 Hard 계획을 만든다. 현재 Admit이지만 soft crossing인 경우에는
   기존 proactive Soft 의미를 유지한다. callback은 최대 1회, broker lock 밖에서 실행한다.
6. 결정에 policy revision, limiting axis, current/request/postimage/limit/deficit과
   eligible-victim scope를 붙인다. 여러 제한은 축별로 유지한다. scalar deficit 하나로
   다른 class/owner의 bytes를 잘못 회수하도록 만들지 않는다.
7. callback 뒤에는 새 lock에서 현재 정책/ledger로 다시 판정한다. 초기/최종 revision을
   구분하고 stale plan을 accounting authority로 사용하지 않는다. revision 추가만으로
   이미 일어난 eviction을 취소할 수 있다고 문서화하지 않는다.
8. 동일 typed decision에서 오류와 diagnostic receipt를 생성한다. 기존 `Capacity(String)`
   caller에 대한 호환 projection은 경계에서만 수행하고, structured 정보는 additive
   diagnostic API로 전달하는 안을 기본으로 한다. 별도 `explain()` 재평가는 금지한다.
   receipt는 호출별 반환값에 결합한다. 경합하는 전역 `last_decision` 조회로 대체하지 않는다.
   구체 API는 시작 시 surface 검토로 고정; 불가피한 breaking change는 명시한다.
   receipt/event emit은 mutex 밖이며 owner 문자열을 무제한 metric label로 쓰지 않는다.

## 검증 계약

- [ ] 불가능한 101 요청: callback 0회, 기존 40 charge/ticket/state 그대로.
- [ ] group hard denial은 soft crossing 여부와 무관하게 유효한 단일 계획; 최대 1회.
- [ ] class/owner/group/global/floor/headroom/scratch별 거절 이유와 실제 판정 일치.
- [ ] 여러 축 동시 초과 시 scope/deficit 보존; 잘못된 victim release만으로 승인되지 않음.
- [ ] callback 중 reconfigure/reentrant release/panic에서도 현재 정책과 fail-closed 원장 유지.
- [ ] 0-byte, overflow, rebuild, unknown class/group, ticket exhaustion에서 불필요한 callback 없음.
- [ ] 초기/최종 revision과 receipt가 실제 판정 시점에 대응; forged reclaimed_bytes로 승인 불가.
- [ ] 기존 reserved floor/Hamilton/static isolation/recharge와 bounded release 성능 계약 유지.
- [ ] preflight 생략 또는 Soft 우선 mutant를 해당 regression이 검출.
- [ ] FUP-007의 0-byte live ticket/group 이동 거절 계약이 preflight 변경 뒤에도
  유지되고, reclaim callback이 그 authority를 우회하지 못한다.

```bash
cargo test --locked -p quanta-memory-platform-core --all-features sep23_fup003 -- --nocapture
cargo test --locked -p quanta-memory-platform-core --all-features --test policy_contracts
cargo test --locked -p quanta-memory-platform-core --all-features --test public_api_surface
```

`sep23_fup003`은 신규 테스트의 예정 prefix다. 실제 `--list` 선택 건수와
negative assertion의 RED→GREEN을 receipt에 남긴다. 위 public probe의 정상
종료(exit 0)는 **버그 관찰**이지 수정본의 PASS가 아니다.

## Handoff / stop 조건

snapshot 전체 clone/owner 전체 scan을 매 reserve에 새로 추가하지 않는다. 순수 predicate
공유와 성능 계약을 동시에 유지할 수 없다면 비용과 대안을 기록하고 범위를 재검토한다.
지금의 reconfigure rejection이나 Hamilton 의미를 함께 바꾸지 않는다.
I에는 전후 denial precedence, public API, policy revision, 필요한 host reclaimer 변경을 전달한다.
