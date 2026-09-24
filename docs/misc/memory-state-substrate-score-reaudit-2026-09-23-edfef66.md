# Memory/state substrate 체크리스트 점수 재감사 — `edfef66`

**역사적 `edfef66` 스냅샷:** 아래의 “현재 HEAD”, “최신 작업트리”, 열린 결함 및
점수는 2026-09-23 당시 소스와 임시 overlay에만 적용된다. 이후 수정·테스트와
적용 범위는 [2026-09-24 재채점](memory-state-substrate-score-reaudit-2026-09-24-db0b03c.md),
현행 qualification 경계는 [계획 상태](../plans/README.md)를 따른다. 이 문서의
당시 점수나 임시 작업트리 결과를 새 HEAD의 receipt로 승격하지 않는다.

기준: [100항목 체크리스트](memory-state-substrate-review-checklist.md), SHA-256 `a92f16aa840ac31bfee3ce35235af625832ca7e2ae480f6a5d1dda66fb1c43d1`. 비교 대상: [최초 점수표](memory-state-substrate-scored-review-2026-09-23.md)의 `028448946caa95676df034481a830c1d3e3b1f33`. 재감사 source: 커밋 `edfef66c3d9dabbe4d01a98754a44d32ef9c940e`, tree `4f1a7299af049e7fa1c0eeefc8e683f01239d462`, annotated tag `qmp-durability-l1-qualified`가 가리키는 **clean commit**. 날짜: 2026-09-23.

감사 당시 공유 작업트리에는 별도 미커밋 코드/문서 변경이 있었다. 이 문서의 **확정 점수는 `edfef66` HEAD blob만** 대상으로 한다. 미커밋 변경은 아래에 별도 잠정 평가하며 합계에 넣지 않는다. 감사 중 실행 입력 digest가 기존 L2 receipt의 `add1f907…`와 달라진 것을 확인했다. 당시 L2 receipt를 그 작업트리 전체의 자격으로 대체하지 않는다.

## 방법과 판정 경계

- 최초 100개 ID를 전부 대조했다. 아래 표의 ID만 새 점수로 교체하고, **나머지 ID는 최초 점수를 유지**한다. 따라서 최초 점수표와 이 변경표를 합치면 100개 ID의 당시 `edfef66` 점수가 결정된다. `범위외` 12개는 계속 평균에서 제외한다.
- 점수는 동일한 [0–100 rubric](memory-state-substrate-scored-review-2026-09-23.md#채점-기준)을 쓴다. 테스트 존재, focused GREEN, clean-source canonical/extraction/scaling, host-pair, deploy/activation을 구별한다. 한 root defect가 여러 ID를 개선해도 독립 결함 여러 건을 고친 것처럼 세지 않는다.
- 근거: [FUP-001~007 완료 기록](../plans/sep-23-followup-misc/README.md), [통합 receipt](../plans/sep-23-followup-misc/RECEIPT.md), `qmp-durability-l1-qualified` tag annotation, [DURABILITY-001](../../tickets/DURABILITY-001-power-loss-finalize-kill.md). 특히 tag는 이 HEAD의 L1 2/2, canonical rail, real extraction, scaling 170 samples와 4/4 negative mutants를 기록한다. `cf2a441`의 host-QBC tuple은 **이 HEAD보다 오래된 소스**이므로 M09의 current-source host GREEN으로 승격하지 않는다.
- 이 재감사는 변경 owner의 코드·테스트·receipt를 대조한 **점수 재판정**이다. HEAD의 전체 rail을 새로 실행한 것이 아니며, 기존 tag/receipt의 raw artifact 전체를 독립 재실행한 것도 아니다. 미커밋 focused 실행은 아래에 따로 적는다.

**집계:** 적용 88항목의 합계는 최초 **4,751/8,800점(53.99/100)**에서 clean HEAD **5,677/8,800점(64.51/100)**으로 **+10.52점**이다. 40개 ID가 상승하고 48개 적용 ID는 유지했으며, `범위외` 12개는 평균에서 제외했다. 큰 변화는 I(spill/recovery) **35.30→62.60**, J(reconfiguration) **42.33→74.00**, B(accounting) **58.60→69.50**이다. 이는 동일 가중 단순 평균이며 release gate가 아니다. 최초 판정의 일부 점수는 아래 테스트 보증 범위를 넘어서 재하향했다.

## 확정 점수 상승 ID

| ID | 기존 → 현재 | 상승 근거와 남은 제한 |
| --- | ---: | --- |
| A04 | 65 → 72 | feature graph와 10개 non-zero capability selector 실행; async waiting 등 미제공 범위는 그대로. |
| B02 | 58 → 84 | `ledger.rs::validate_exact`가 class/owner/context kind vector를 교차검증; kind-swap negative와 mutant RED. |
| B05 | 72 → 79 | max-scale `u64::MAX`와 checked-total 경계 추가; 전체 물리 계량 보장은 아님. |
| B07 | 18 → 84 | 0-byte 권한 수명 수정에 더해 `MAX_LIVE_TICKETS_PER_BROKER`, owner/group key 길이 한도 및 cap negative 테스트 추가. 논리 개수 한도이지 RSS cap은 아님. |
| B10 | 65 → 75 | recharge/batch-release의 kind-vector 정확성 negative 추가; 미제공 split/merge는 점수화하지 않음. |
| C01 | 68 → 76 | 공유 preflight/attempt의 byte-limit 불가능 요청 거절과 randomized agreement/mutants. live-ticket **개수** 상한은 preflight에 없어 전체 불가능 요청의 callback-free 보장은 아님. |
| C02 | 71 → 78 | global/class/owner/group/floor/scratch postimage의 shared predicate와 boundary tests; host 경쟁 부하는 별도. |
| C05 | 16 → 84 | 0-byte live ticket owner/group을 reconfigure에서 보존·거절; committed/pending/Drop negative. |
| D04 | 72 → 78 | `DurableRestoreClaim`도 move-only caller authority로 제한; 기존 charge release single-use 규칙 유지. |
| D07 | 63 → 82 | explicit/Drop/rebuild deferred 권한을 0-byte까지 보존; drain/re-push 관련 negative mutants. |
| E01 | 67 → 75 | mutation permit·rebuild·durable finalize의 postimage negative 확대; production 모든 schedule의 증명은 아님. |
| E03 | 70 → 78 | prepared/committed decode panic 후 source/claim 복구 negative 추가; 일반 allocator abort 보장은 없음. |
| E04 | 67 → 75 | reclaim 이후 현재 policy/ledger로 재판정하는 shared admission path와 테스트 강화. |
| E07 | 66 → 81 | exact claim/generation/finalize retry 충돌과 terminal-proof-first 재시도 oracle 추가. |
| E08 | 72 → 78 | durable ambiguous outcome/reconcile·claim 관련 negative 확대; generic error 구조화(K06)는 그대로. |
| F02 | 70 → 78 | forged `reclaimed_bytes`가 admission을 승인하지 않는 negative proof. |
| F07 | 32 → 48 | 불가능한 101-byte 요청의 선제 회수 제거와 soft/hard 계획 테스트는 GREEN. 그러나 live-ticket 16,384개 포화는 ledger의 `reserve`에서만 거절되고 broker preflight는 ID 고갈만 본다. 따라서 불가능한 추가 요청에서 reclaimer가 먼저 호출될 수 있는 소스 수준 공백이 남는다. 일반 scan/I/O 상한도 미확립. |
| I01 | 60 → 79 | 실제 SIGKILL/cold reopen과 finalize 양쪽 경계 L1; physical power loss는 미검증. |
| I02 | 35 → 80 | record disposition, backend-bound move-only claim과 generation/attempt authority; 모든 host transfer semantics는 별도. |
| I03 | 42 → 65 | Redb transaction 전/후 finalize kill로 QMP linearization 입증; dependency 내부 sync/power-loss까지는 아님. |
| I04 | 18 → 70 | 한 record의 recovery 단일 claimant와 exactly-one publication negative는 있다. 체크리스트가 요구하는 RAM/disk/staging 동시 점유·예산의 독립 oracle은 없다. |
| I06 | 43 → 52 | decode panic/restore claim 복구가 강화됐으나 decode/copy scratch 선승인·peak bound는 없음. |
| I07 | 15 → 68 | exact `Reserved/Present/Finalized` disposition, hole regression과 reopen 테스트는 있다. ambiguous commit 후 `Present` 판정은 record 존재/disposition을 확인하지만 예상 payload의 내용·digest까지 대조하지 않는다. 데이터 손실 재현 판정은 아니다. |
| I08 | 17 → 76 | persisted exclusive recovery claim과 두 store의 동일 프로세스 경합은 테스트했다. SIGKILL/reopen은 backing 수준이며 store+ledger 전체 재시작 tuple은 아직 없다. |
| I09 | 35 → 48 | schema v2 migration/fail-closed legacy 처리 추가; checksum·decode length/cost-model envelope는 미해결. |
| J02 | 68 → 75 | limit 축소/reconfigure denial의 revision·ledger 불변 negative 확대. |
| J03 | 15 → 84 | 제거된 group 탓에 0-byte ticket의 최종 release가 막히던 경로 수정. |
| J04 | 53 → 75 | config/scheduler 교체에 0-byte 기존 권한 검사를 포함하고 reject postimage를 검증. |
| J05 | 15 → 84 | positive usage만 보던 scope 삭제 검사에 live zero-byte authority 포함. |
| J06 | 49 → 72 | deferred 권한 queue는 live-ticket cap에 종속되고 개별 drain critical section은 1,024개로 제한된다. `finish_rebuild` 호출 전체는 모든 batch를 순회하므로 전체 작업량·tail latency 상한 증거는 없다. |
| K03 | 70 → 76 | durable claim-bearing caller surface를 move-only로 제한; generic external SPI는 charge authority 아님. |
| K05 | 62 → 78 | exact HEAD feature matrix/advertised capability selector와 standalone extraction GREEN; production host feature 선택은 별도. |
| L03 | 45 → 56 | full scaling verifier 170 samples를 현재 HEAD에서 실행; 성공/busy/deny/reclaim/spill별 운영 tail receipt는 없음. |
| L04 | 28 → 36 | accepted reconfigure provenance는 bounded active record + 10k/mutant oracle로 개선. **event-sink 재진입은 이 clean HEAD에서 여전히 열려** 있어 낮은 점수를 유지. |
| M01 | 64 → 84 | class/context/owner kind negative, broken-fake rejection, 다수 mutant RED와 clean-source rail. |
| M02 | 64 → 78 | compile-fail/doc selector의 현재 HEAD canonical 실행; 모든 외부 caller 조합은 아님. |
| M04 | 45 → 59 | durable ACK/decode/panic/claim fault injection 확대; Hash/Eq/Clone/Drop과 fallible allocation 전체 matrix는 아님. |
| M05 | 40 → 68 | 실제 SIGKILL/reopen 3개와 finalize Before/AfterCommit L1 2개는 `RedbSpillBacking` 직접 호출이다. `RevocableStore`+ledger의 ACK/source-release/restore/delete 전 경계 재시작 matrix는 없다. |
| M06 | 47 → 70 | 현 HEAD scaling verifier 170 samples GREEN 기록이 있다. verifier는 필수 matrix·반복·증가율과 기본 p99 ≤ 1초 상한을 검사한다. 다만 operation/outcome별 production SLO는 없고 이 감사에서 raw artifact를 재실행하지 않았다. |
| M10 | 35 → 65 | annotated tag가 이 HEAD의 source/tree와 canonical/extraction/scaling/mutant 결과를 요약한다. 일부 raw extraction/host 경로는 현 환경에서 조회되지 않고, exact HEAD host-QBC·hosted CI/deploy는 없다. tag 요약을 전체 재현 가능한 raw receipt로 취급하지 않는다. |

기존 점수에서 **하락 판정한 ID는 없다.** 새 코드를 넣었다는 사실만으로 A05/B06, E02/E05/F06, F01/H01, F05/I05/I06/I09, K06, L05/L06, M07 등을 자동 상향하지 않았다. 아래 미커밋 작업이 일부를 다루지만, clean HEAD 판정과 합치지 않는다.

## 미커밋 작업트리의 잠정 개선 — 확정 합계 제외

첫 재감사 중 공유 작업트리의 `git diff --binary` hash가 `d76d42…`에서 `b1745d…`로 변했다. 그 뒤 `b1745d…`가 유지되는 동안 아래 focused 실행은 통과했다. 당시 Rust/Cargo/script 입력 digest `185dd3329b00d685b866c992c03e50b1bcaa1f8a1097b0310275abb05d4dd2fa`는 기존 L2 receipt의 `add1f907…`와 다르다. 이는 **그 순간의 작업트리 실행**이며 아래 후속 재감사의 최신 snapshot은 아니다.

| ID | 관찰한 변화 | 잠정 판정과 보류 조건 |
| --- | --- | --- |
| A05/B06 | `ByteSized` 문서에 내부 가변/shared payload의 보수적 상한 또는 명시적 recharge/replace 책임을 추가. | 신뢰 경계는 명확해지지만 host custom type oracle 없음; clean HEAD 점수 유지. |
| F06/L04 | same-broker/cross-thread reclaimer single-flight, event-sink recursion suppression 코드. `cargo test --locked -p quanta-memory-platform-core --all-features reentry`: 3 selected GREEN. | callback storm 경계 개선. 외부 callback 시간/작업량은 여전히 강제 제한 불가; clean HEAD 점수 유지. |
| E02/E05/M04 | read-side caller `Clone`/`Ord`/`CanonicalKey` panic을 guard drop 뒤 다시 전파하는 경계. retained-map focused panic test 1 selected GREEN. | mutex poison 경감이나 느린 caller work는 여전히 lock 아래 있고 payload/transcript 개별 panic matrix는 미확인; clean HEAD 점수 유지. |
| I03 | Linux ARM64 `pwrite64`/`fdatasync` EIO subrail 기록. | 마지막 기록 digest `add1f907…`와 감사 시점 실행 입력 digest가 일치하지 않음. 현재 작업트리 자격으로 승격하지 않음; 물리 power-loss는 별도. |

## 추가 재감사: 최신 작업트리 잠정 점수

후속 감사의 **Rust/Cargo 입력 digest**는 `58c2cc013efa36b66328bdeadf0f5f2ec483768f59ac1c43b06d9662ee893769`로 테스트 전후 동일했다. 이 snapshot에서 `cargo test --locked --workspace --all-features --quiet`는 exit 0: core 245, facade 282, testkit 82, 정책/공개 API/E2E/kill-reopen/doc 테스트 모두 통과했다. 개별 reentry 3/3과 retained-map panic 1/1도 이전 집중 실행에서 통과했다. 다만 PM 파일은 별도 사용자 작업으로 바뀌고 있으며, 이 전체 작업트리는 아직 clean commit이 아니다.

**직전 제품 후보의 정식 rail은 GREEN:** 중간 snapshot에서는 Python 3.9의 `typing.TypeAlias` import 때문에 PM 단계에서 실패했으나, 이후 공유 작업트리에서 이 오류가 수정됐다. Rust/Cargo digest `58c2cc…`가 유지된 동안 `scripts/check-workspace.sh`를 재실행해 exit 0을 확인했다. format, ShellCheck, workspace graph/policy, Python contract, all-feature 테스트, 10개 feature runtime path, 228개 doc selector, PM lint, deny가 통과했다. 실행 전후 `git diff --binary` hash는 `1f9c95cb…`로 같았다. **이 보고서 자체는 rail 이후 수정됐고, 그 문서-only 변경의 rail은 재실행하지 않았다.** 이는 미커밋 작업트리의 제품 rail이며 clean commit/extraction/scaling/host/L2의 동일 소스 receipt로 확대하지 않는다.

아래는 clean HEAD 점수에서 **최신 작업트리 후보**로 바뀌는 항목이다. 그 외 91개 ID는 위 clean HEAD 점수를 유지한다. 현재 후보 합계는 적용 88항목 **5,771/8,800점, 65.58/100점**으로 clean HEAD 64.51보다 **+1.07점**이다. 이는 제품 qualification 점수가 아니라 dirty snapshot의 계약/증거 점수다. canonical은 통과했지만 미갱신 scaling/extraction/host/L2 증거가 남는다.

| ID | clean HEAD → 작업트리 잠정 | 추가 판정 |
| --- | ---: | --- |
| A05 | 42 → 52 | 내부 가변/shared `ByteSized`는 생존 기간 보수적 상한 또는 explicit recharge/replace 책임이라고 문서화; host custom type 검증 없음. |
| B06 | 45 → 58 | 비용 공식의 사후 변경 규칙을 문서/trait에 추가; 강제 가능한 runtime 불변성이나 host oracle은 없음. |
| E02 | 36 → 48 | `Clone`/`Ord`/`CanonicalKey` panic 경계는 개선됐지만 일부 caller 작업은 여전히 state mutex 아래 실행되고 느린 callback의 시간 상한 없음. |
| E05 | 38 → 42 | 일부 read-side panic을 mutex guard 정상 해제 후 전파하고 retained-map negative가 통과했다. 그러나 payload `PayloadRef::Eq`, transcript `SessionId::Ord` 등은 여전히 lock 안에서 호출된다. 기존 transcript panic 테스트는 poison된 mutex를 내부에서 회수할 뿐 public store 가용성을 증명하지 않는다. |
| F06 | 35 → 72 | same-broker 재진입 typed reject 및 cross-thread reclaimer single-flight; reentry 3/3과 전체 Rust rail 통과. 비협조적 callback 자체를 중단하지는 못함. |
| L04 | 36 → 66 | bounded provenance에 event-sink single-flight/drop-on-overlap을 추가. 외부 sink 실행시간·작업량은 제한하지 않음. |
| M04 | 59 → 64 | retained-map `Clone` panic 후 store 가용성 negative 추가; 전체 Hash/Eq/Clone/Drop/adapter failure matrix는 아님. |
| M06 | 70 → 60 | control hot path가 바뀐 현재 후보의 full scaling verifier 미실행; 이전 HEAD 170-sample receipt는 구소스 증거. |
| M10 | 65 → 58 | 현 작업트리 canonical GREEN이나 clean extraction/scaling/host tuple과 current-source L2 receipt 없음. HEAD tag의 과거 GREEN은 유지되지만 이 후보의 receipt는 아님. |

**우선 조치:** 소스 freeze 후 scaling/extraction/host tuple 및 필요한 L2를 동일 digest로 갱신한다. live-ticket cap의 단일 회수·의미적 재측정, payload/transcript panic 후 public 가용성, store+ledger 전체 재시작 matrix를 별도 테스트로 입증한다. 이 조치 전에는 작업트리 65.58을 release-qualified 점수로 표현하지 않는다.

## 병렬 테스트 보증 감사

**결론: 아니오.** 점수는 계약 구현과 증거 강도를 함께 판단한 리뷰 점수이며, 테스트 통과가 숫자 자체를 수학적으로 보장하지 않는다. 아래의 검증된 경계에는 회귀 테스트가 있지만 체크리스트 전체, 특히 host·물리 자원·재시작 조합을 보증하는 통합 rail은 아직 없다. `scripts/`·`crates/`·`tools/pm/`에서 이 100개 체크리스트 ID 또는 점수표를 검증하는 자동 매핑도 확인되지 않았다. 이 표의 clean/작업트리 구분을 유지한다.

| 계약·ID | 있는 oracle/실행 증거 | 아직 보증하지 않는 경계 |
| --- | --- | --- |
| 원장 kind 벡터·0-byte 권한(B02/B07/C05/J03/J05) | `domain/ledger.rs`의 exact cross-axis validation과 live-ticket cap 테스트, `testkit/conformance/suites.rs`의 독립 bucket×kind 모델·broken fake reject, `api/control.rs`의 committed/pending/Drop/rebuild/group 제거 negative. clean HEAD 기록과 작업트리 전체 Rust rail GREEN. | allocator/RSS 물리 cap, 모든 host custom `ByteSized`의 lifetime cost. |
| 불가능 요청·bounded reclaim(C01/F07) | `api/control.rs::sep23_fup003_terminal_matrix_never_fires_callback`, randomized preflight/attempt agreement, soft/hard 계획 테스트. | `sep23_fup003_ticket_exhaustion_blocks_without_callback`은 `u64` ID 고갈을 합성한다. live-ticket **개수** cap 16,384의 public broker+reclaimer 음성 테스트는 없다. `ledger.rs::reserve`만 그 개수를 검사하므로 압박 상황의 선제 callback 가능성은 소스 수준 판정이며 실행 재현은 아직 없다. |
| durable disposition·복구 배타성(I02/I04/I07/I08) | `spill/backings/redb.rs`의 reserved-hole/claim/generation negative, `revocable_store/service.rs::sep23_fup002_concurrent_recovery_has_exactly_one_owner`; `spill_redb_kill_reopen.rs` 3개와 `spill_redb_finalize_kill.rs` 2개의 실제 child SIGKILL/cold reopen. | SIGKILL 테스트는 `RedbSpillBacking` 직접 호출이며 `RevocableStore`+ledger/host의 전체 재시작은 아니다. 동일 프로세스 두-store race를 process-kill race로 확대할 수 없다. |
| ambiguous commit reconciliation(I07/M05) | record ID별 `Present/Absent/Finalized`와 disposition 일치 검증, ACK-loss/finalize 테스트. | `resolve_record`는 해당 ID의 payload 존재와 disposition을 확인하지만 예상 bytes/digest 일치를 비교하지 않는다. `reconcile_durable_spill`은 `Present`면 source를 retire한다. 내용 불일치로 인한 실제 손실은 **미재현**이며, ID·generation·내용 계약의 독립 oracle이 필요하다. |
| caller panic·재진입(E02/E05/F06/L04/M04) | 작업트리의 broker reentry 3개, retained-map Clone panic 후 재사용 1개, transcript Ord panic 원장 보존 테스트; focused와 전체 Rust rail GREEN. | payload `PayloadRef::Eq`, transcript `SessionId::Ord`는 일부 mutex 안에서 실행된다. transcript 테스트의 내부 `PoisonError::into_inner`는 caller가 public API로 계속 쓸 수 있다는 증거가 아니다. callback 시간 상한·panic 후 gate 해제·외부 side effect rollback도 미검증. |
| 성능·재현 receipt(M06/M09/M10) | clean HEAD tag에 170-sample scaling, canonical, extraction, mutants 요약. 현재 작업트리 canonical과 all-feature Rust rail GREEN. | scaling verifier는 기본 p99 ≤ 1초 상한은 검사하지만 operation/outcome별 production SLO는 없다. current-source host-pair raw QBC는 확인되지 않았고, 작업트리의 scaling/extraction/L2 receipt는 갱신되지 않았다. L2의 기존 raw EIO receipt는 이전 source digest `add1f907…`에 묶인다. |

테스트 추가 순서는 **(1) live-ticket cap public API에서 no-op 회수 1회 후 원장 불변·거절과 실제 0-byte ticket 회수 후 승인, (2) payload·transcript caller panic 뒤 public API 재사용, (3) store+ledger 전체 프로세스 재시작 및 ACK/source-release/restore/delete 경계, (4) expected content identity reconciliation, (5) 동일 소스 scaling·host·L2 receipt**다. (1)은 ticket 수명이 byte 사용량과 독립적이며 host가 실제 ticket을 회수할 수 있다는 사실을 검증한다. (3)–(5)는 현재 통과한 backing/unit rail을 대체하지 않고 범위를 넓힌다.

## 남은 낮은 점수와 gate

- **`edfef66` 당시 clean HEAD의 열린 결함:** F06 reclaimer와 L04 event-sink 재진입은 당시 bounded 실행으로 재현된 경계다. E02/E05 read-side caller panic/lock 경계는 당시 소스 수준 판정이다. 이후 수정·테스트는 위의 2026-09-24 재채점을 따른다.
- **선택 기능 계약:** I05 disk/record/I/O/codec budget, I06 decode/copy scratch peak, I09 content integrity/length bound. L1 SIGKILL GREEN은 이들을 닫지 않는다.
- **host 의미·운영:** A05/B06 caller-reported cost, F01/H01 victim eligibility, D06/D08 close/pin/owner shutdown, M07 장시간 phase-change soak가 남는다. 실제 host 사용 여부에 따라 우선순위를 정한다.
- **`edfef66`-source host:** 당시 HEAD의 exact QMP/host tuple QBC, hosted CI, 배포·활성화는 미검증. `cf2a441` receipt를 `edfef66`로 복사하지 않는다.

따라서 이 재감사는 기존 P1 세 root가 해결되고 상당한 점수 상승이 있음을 인정하지만, **100/100이나 host activation**을 주장하지 않는다.
