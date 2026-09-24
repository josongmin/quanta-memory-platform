# QMP memory/state substrate — 100항목 점수 평가

**이 문서는 `0284489` 기준점이다.** 현재 `db0b03c` 재채점과 in-memory 적용 범위는 [2026-09-24 재감사](memory-state-substrate-score-reaudit-2026-09-24-db0b03c.md)를 참조한다. 이 기준점, [중간 `edfef66` 변경표](memory-state-substrate-score-reaudit-2026-09-23-edfef66.md), 현재 변경표를 순서대로 적용하면 100개 ID의 숫자가 결정된다. 미커밋 작업트리는 그 결과에 포함되지 않는다.

평가일: 2026-09-23. 대상: `main@028448946caa95676df034481a830c1d3e3b1f33` (tree `f983669731805a330200213bad7bf1bd5573f308`). 기준: [100항목 체크리스트](memory-state-substrate-review-checklist.md), SHA-256 `a92f16aa840ac31bfee3ce35235af625832ca7e2ae480f6a5d1dda66fb1c43d1`. 항목별 source 위치와 실행 기록은 [1차 감사](memory-state-substrate-parallel-audit-2026-09-23.md) 및 [재감사](memory-state-substrate-gap-reaudit-2026-09-23.md)에 있다. 이 문서가 두 감사의 상태 분류를 **점수 판정으로 대체**한다. 제품 코드는 수정하지 않았다.

## 채점 기준

이 점수는 시장 제품과의 실측 순위가 아니라, 이 체크리스트가 요구하는 **SOTA 수준의 계약 완성도와 증거 강도**를 현재 HEAD에 대해 합친 평가다. SOTA 목표는 단일 권한·fail-closed accounting, 재진입과 고갈 시 bounded progress, durable 사본의 배타적 custody, semantic/physical 자원 경계, 독립 negative oracle, 동일 소스의 crash/host/성능 receipt이다. 특정 cache 알고리즘 채택은 점수 요건이 아니다.

| 점수대 | 판정 기준 |
| --- | --- |
| 0–24 | 계약 핵심을 깨는 재현 결함 또는 해당 권한의 소실/중복. 결함이 여러 ID에 걸쳐도 원인은 별도로 센다. |
| 25–39 | 재현되거나 소스에서 특정된 조건부 실행 결함, 또는 핵심 안전·진행성 제약 부재. 배포 사고를 추정하지 않는다. |
| 40–59 | 경계·일부 구현은 있으나 요구 행동 계약의 중요한 부분이나 독립 oracle이 빠짐. 행동을 약속하면서 문서만 있는 경우도 여기서 멈춘다. |
| 60–74 | 관련 구현과 테스트/계약 경계가 확인됨. 이 HEAD에서 해당 항목의 충분한 negative·통합 rail을 완료한 것은 아님. |
| 75–89 | 미지원·비보장 자체가 평가 대상인 범위 계약이 명확하거나 일부 focused 실행이 있으나 동일 소스의 전체 qualification은 없음. 적용 항목에서 정합성 결함이 있으면 이 점수대 불가. |
| 90–100 | 현재 소스에 대해 실패 주입, 관련 feature·host 조합, 운영/성능 receipt까지 해당 계약을 입증. 이번 평가에는 해당 항목 없음. |

재현된 정합성 결함은 최대 24점, 재현된 조건부 진행성/가용성 결함은 최대 39점, 소스 수준에서만 특정된 결함은 최대 49점, 미실행 **행동 구현** 관찰은 최대 74점이다. 미지원·비보장 범위의 명확성은 행동 구현과 별도로 평가한다. 숫자는 정확한 신뢰확률이 아니라 **비교 가능한 리뷰 판정**이며, 1–2점 차이에 의미를 부여하지 않는다. `범위외` 항목도 0–100 점수를 남겼다. 이는 미제공 capability의 **범위 선언과 잘못된 보장 부재**를 평가한 숫자이지 그 기능의 구현 점수가 아니므로 적용 항목 평균에서 제외한다. 기능을 제공하기 시작하면 즉시 재평가한다.

이 평가에서 `적용`은 QMP 또는 명시된 host 접점의 현재 계약이며, `범위외`는 선택 기능을 제공하지 않는다는 뜻이다. `범위외`는 무결함의 동의어가 아니다. 각 행의 근거는 코드/문서 관찰 또는 명시된 focused 실행이며, 기존 테스트 파일의 존재를 현 HEAD 통과로 계산하지 않았다.

## 항목별 결과

| ID | 적용 | 점수 | 근거와 점수를 제한한 조건 |
| --- | --- | ---: | --- |
| A01 | 적용 | 75 | `README-detailed.md`가 semantic/allocator/RSS/disk 단위를 구분; 실제 물리 cap은 약속하지 않음. |
| A02 | 적용 | 78 | process-local 등록 상태 범위를 명시; 외부 allocation 통제 주장은 없음. |
| A03 | 적용 | 68 | budgeted builder와 broker-bound authority 확인; foreign broker 이동 negative 실행 없음. |
| A04 | 적용 | 65 | feature 범위 문서화; capability별 오류/host 조합의 현재 receipt 없음. |
| A05 | 적용 | 42 | caller `ByteSized` 및 내부 가변 payload의 사후 과금 책임이 불명확. |
| A06 | 적용 | 74 | 엔진 결과 유효성·activation은 host 책임이라고 분리. |
| B01 | 적용 | 70 | pending은 ticket 상태이며 kind/semantic totals에 포함; 별도 pending projection은 운영 문제. |
| B02 | 적용 | 58 | 원장 축 합계는 검증하나 class/context kind 교환을 잡는 독립 oracle 없음; 원장 손상 재현은 아님. |
| B03 | 적용 | 70 | store publication 이전 charge; caller 선할당은 semantic cap 밖. |
| B04 | 적용 | 68 | checked preimage/rollback 경로 관찰; fallible allocation 전 범위 negative 실행 없음. |
| B05 | 적용 | 72 | checked postimage와 비용 산술 확인; 전체 단위변환 rail 미실행. |
| B06 | 적용 | 45 | V1 공식·원금 환급은 명시; 내부 가변 payload 규칙이 미완성. |
| B07 | 적용 | 18 | 0-byte ticket의 group 수명 결함 재현; 별도 ticket-count/metadata 상한도 없음. |
| B08 | 적용 | 67 | payload dedup 재사용 경로 확인; owner 귀속/참조 변화 negative rail 미실행. |
| B09 | 적용 | 53 | net postimage charge는 있으나 교체 중 managed overlap/physical peak 경계가 불충분. |
| B10 | 적용 | 65 | 제공되는 recharge/batch release 권한 경로 확인; 미제공 split/merge는 강제하지 않음. |
| C01 | 적용 | 68 | typed capacity denial과 유한 retry; 불가능한 요청의 선제 회수는 F07에서 감점. |
| C02 | 적용 | 71 | global/class/owner/group postimage를 broker 경계에서 검증. |
| C03 | 적용 | 66 | 정적 weighting·burst와 fairness 한계를 문서화; host admission 보장 증거 없음. |
| C04 | 적용 | 68 | floor/band 유효성 검사; 회수 불가능 상태의 완전한 진행성 oracle 미실행. |
| C05 | 적용 | 16 | 0-byte live ticket의 발급 group 제거 후 release 불가 재현. |
| C06 | 범위외 | 80 | async waiting/queue 미제공을 명시; 대기 정책 구현 점수는 아님. |
| C07 | 적용 | 68 | 정적 burst cap과 반환 경로 확인; 차입 회수 실패의 host 시나리오 미검증. |
| C08 | 적용 | 49 | throughput fairness 비보장은 명시하지만 zero-floor group의 admission starvation 경계는 불충분. |
| D01 | 적용 | 67 | 관리 객체 charge와 caller-owned clone 분리; guard 수명 범위의 집중 실행 없음. |
| D02 | 범위외 | 80 | retained map은 borrowed guard 대신 owned copy 반환; pin은 별도 의미. |
| D03 | 적용 | 72 | semantic refund와 allocator 반환·caller clone을 문서상 분리. |
| D04 | 적용 | 72 | non-Clone release authority와 실패 시 복구 경로 확인. |
| D05 | 적용 | 67 | payload ref 단조 증가/slot 정리; stale ID 전체 조합의 negative 실행 없음. |
| D06 | 적용 | 51 | per-store close/Drop은 있으나 owner-wide child 종료는 host choreography 미계약. |
| D07 | 적용 | 63 | deferred Drop/rebuild 경로는 존재하나 0-byte group 변경에서는 실패하므로 전면 충족 아님. |
| D08 | 적용 | 52 | close는 terminal; outstanding pin/pending과 host shutdown 순서 계약이 불완전. |
| E01 | 적용 | 67 | mutation/close gate와 preimage 복구 경로 확인; 전체 동시성 rail 미실행. |
| E02 | 적용 | 36 | caller `Clone`/`Ord`/`CanonicalKey` 등이 store mutex 아래 실행; poison/진행성 위험은 소스 수준. |
| E03 | 적용 | 70 | RAII state plan/preimage 복구 확인; 중단점 전 범위 negative 실행 없음. |
| E04 | 적용 | 67 | 최종 재측정/version 비교 경로 확인; generation/pin 조합의 전체 oracle 미실행. |
| E05 | 적용 | 38 | 일부 encoder panic은 포착하지만 다른 caller panic은 mutex poison 가능; abort 복구는 미약속. |
| E06 | 범위외 | 80 | async Future/cancellation API 미제공; host wrapper가 별도 책임. |
| E07 | 적용 | 66 | retryable mutation의 identity 경로 관찰; callback exactly-once 보장은 하지 않음. |
| E08 | 적용 | 72 | pending/reconcile 조회 API로 durable unknown과 busy를 구분 가능; generic error는 K06. |
| F01 | 적용 | 53 | revocable/unique 상태는 다르게 다루지만 host generic reclaimer의 victim 분류 계약 부족. |
| F02 | 적용 | 70 | callback 보고량에 의존하지 않고 ledger를 재측정. |
| F03 | 적용 | 55 | concrete store가 pin/charge를 검사; generic host callback 후보 권한 계약은 미완성. |
| F04 | 범위외 | 80 | 별도 unused-capacity grant API 미제공. |
| F05 | 적용 | 48 | release는 새 admission에 의존하지 않지만 spill codec/scratch peak 한도 없음. |
| F06 | 적용 | 35 | same-broker reclaimer 4회 중첩을 bounded probe로 재현; 실제 stack exhaustion은 미실행. |
| F07 | 적용 | 32 | 불가능한 101-byte 요청이 40 bytes를 먼저 회수; soft/hard plan 누락도 재현. |
| F08 | 적용 | 64 | host batch 축소·거절 경계는 문서화; 실제 host 선택/receipt 없음. |
| G01 | 적용 | 70 | allocator sample은 advisory, semantic ledger와 분리. |
| G02 | 범위외 | 80 | OS/cgroup/PSI physical sampler 미제공; zero 대체 주장 없음. |
| G03 | 적용 | 48 | 자동 semantic pressure 경로에 hysteresis/cooldown 및 진동 실험 없음. |
| G04 | 적용 | 68 | denial 중 release 가능; physical OOM escalation은 host 책임. |
| G05 | 적용 | 76 | semantic cap을 RSS hard bound로 주장하지 않는 경계가 명확. |
| G06 | 범위외 | 80 | adaptive physical sampling policy 미제공; hot path hidden sampler 없음. |
| H01 | 적용 | 55 | revocable과 transcript 의미는 다르나 host의 recomputable/unique 분류는 미계약. |
| H02 | 범위외 | 80 | 엔진 결과 cache identity 생성은 host 책임; QMP가 해당 capability를 제공하지 않음. |
| H03 | 범위외 | 80 | TTL/borrowed read guard capability 미제공. |
| H04 | 적용 | 68 | byte-target reclaim과 pin block 경로 확인; workload 실행 없음. |
| H05 | 범위외 | 80 | learned eviction sketch/ghost policy 미제공. |
| H06 | 범위외 | 80 | single-flight registry 미제공; host 중복 계산 비용은 별도. |
| H07 | 적용 | 65 | transcript replay 증거가 payload truncate 뒤에도 유지; horizon/saturation 전체 rail 미실행. |
| I01 | 적용 | 60 | persisted claim/recovery slot 존재; 재차 crash 보장은 actual process matrix로 미검증. |
| I02 | 적용 | 35 | record ID는 안정적이나 transfer/attempt/claim identity 계약이 두 durable 결함을 막지 못함. |
| I03 | 적용 | 42 | transaction receipt와 ambiguous ACK probe는 있으나 sync·power-loss failure model 미규정. |
| I04 | 적용 | 18 | 단일 durable record를 두 store가 각각 `Restored`해 resident 사본/charge 중복 재현. |
| I05 | 적용 | 32 | disk bytes·record count·in-flight I/O·codec scratch 상한 미확립. |
| I06 | 적용 | 43 | resident charge는 durable delete보다 앞서지만 decode/copy peak는 선승인되지 않음. |
| I07 | 적용 | 15 | 미기록 reserved ID가 후속 finalize watermark로 `Finalized` 오판됨을 재현. |
| I08 | 적용 | 17 | 재시작 복구 후보의 배타적 신규 claimant 부재; 중복 복구 재현. |
| I09 | 적용 | 35 | durable envelope/version/checksum/decode length bound 및 threat-model 계약 없음. |
| I10 | 적용 | 56 | QMP는 제공된 roots를 검증; 완전한 snapshot/activation barrier는 host 미검증. |
| J01 | 적용 | 54 | report revision은 있으나 각 charge decision/receipt의 정책 version 식별 없음. |
| J02 | 적용 | 68 | usage 아래 limit 축소 reject 경로 확인. |
| J03 | 적용 | 15 | group 삭제 후 0-byte 최종 반환 불가; pending-Drop은 추가 소스 위험. |
| J04 | 적용 | 53 | config/scheduler 원자 교체는 있으나 0-byte 기존 권한 예외로 계약 부분 충족. |
| J05 | 적용 | 15 | positive owner usage만 검사해 live 0-byte scope 삭제 허용. |
| J06 | 적용 | 49 | rebuild fence/deferred drain 존재; backlog/work bound 및 0-byte drain 결함. |
| K01 | 적용 | 70 | builder 결합과 non-Clone charge authority 확인. |
| K02 | 적용 | 53 | owned copy/replace/remove/close는 있으나 host batch의 선 materialization 제한 미확립. |
| K03 | 적용 | 70 | charge-bearing backing authority는 sealed; generic external SPI는 비권한 경계. |
| K04 | 적용 | 70 | core executor/hidden thread 의존 없음; 동기 close 책임 명시. |
| K05 | 적용 | 62 | feature-gated export와 unbudgeted opt-in 분리; 현 HEAD feature matrix 미실행. |
| K06 | 적용 | 47 | generic errors에 scope/bytes/revision/retryability 구조가 부족; 조회 API가 일부 보완. |
| K07 | 범위외 | 80 | FFI/foreign-buffer deallocator·lifetime API 미제공. |
| L01 | 적용 | 60 | broker lock의 원자 observation/revision은 있으나 per-mutation sequence 없음. |
| L02 | 적용 | 66 | ledger kinds/charged pending 구분; spilled inventory와의 관계는 일부 경계. |
| L03 | 적용 | 45 | benchmark tail sample은 있으나 성공/busy/deny/reclaim/spill별 현 HEAD tail receipt 없음. |
| L04 | 적용 | 28 | event sink→observe 재진입 bounded 재현; reconfigure provenance/clone 비용도 무제한 증가. |
| L05 | 적용 | 46 | pending ID/pin count는 있으나 age·action·cleanup backlog 진단 부족. |
| L06 | 적용 | 49 | owner totals/live-ticket count는 있으나 handle inventory와 quiescent leak 구분 부족. |
| M01 | 적용 | 64 | reference trace/broken-fake rejection 테스트 존재; 현 HEAD 실행하지 않음. |
| M02 | 적용 | 64 | compile-fail 계약 테스트 존재; 현 HEAD 실행하지 않음. |
| M03 | 적용 | 62 | Loom 축약 모델 범위 명시; production lock schedule 전체 증명 아님. |
| M04 | 적용 | 45 | 일부 panic/poison 주입은 있으나 caller trait/fallible allocation 전체 matrix 없음. |
| M05 | 적용 | 40 | Redb drop/reopen 3개는 현 HEAD 통과; 실제 process 종료/재시작 경계 matrix 없음. |
| M06 | 적용 | 47 | scaling matrix/검증 스크립트 존재; 현 HEAD 실행/성공률·busy receipt 없음. |
| M07 | 적용 | 30 | pin·reconfigure·reclaim 장시간 phase-change soak 없음. |
| M08 | 범위외 | 80 | learned cache 정책 선택 미제공; trace 비교가 필수 아님. |
| M09 | 적용 | 30 | 이전 QMP/host 쌍의 receipt는 있으나 이 HEAD의 host E2E/activation 아님. |
| M10 | 적용 | 35 | 이전 소스 receipt는 현재 source identity를 증명하지 못함; 재현 정보 새로 필요. |

## 집계와 gate

`범위외` 12항목은 숫자를 기록하되 평균에서 제외한다. 적용 88항목 합계 **4,751/8,800점**, 단순 평균 **53.99/100점**이다. 최저 영역은 I(spill/recovery) **35.30**, J(reconfiguration) **42.33**, M(verification/host) **46.33**점이다. 평균은 위치 파악용이지 release gate가 아니다. 같은 결함이 여러 항목에 영향을 미치므로 결함 수와 실패 원인을 점수 합계에서 역산하면 안 된다.

- **Core gate: 미통과.** 확인된 0-byte ticket scope/release 권한 결함이 있다. 확인된 P0은 없다.
- **Durable-enabled gate: 미통과.** reserved hole 오판과 duplicate recovery claimant가 별개 원인으로 재현됐다. `spill-redb`를 제공하지 않는 구성의 판정과 분리한다.
- **Host/운영 gate: 미판정.** 현재 소스의 full rail, process-kill recovery, host pair, scaling/soak, activation receipt가 없다.

우선순위 실행 계약은 [FUP 계획](../plans/sep-23-followup-misc/README.md)에 있다. 해당 계획의 완료나 일부 focused GREEN만으로 이 표의 90점대나 host gate 통과를 선언하지 않는다. 수정 후에는 변경된 owner 항목과 연쇄 영향 항목을 동일 HEAD에서 재채점한다.
