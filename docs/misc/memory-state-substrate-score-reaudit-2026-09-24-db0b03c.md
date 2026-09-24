# Memory/state substrate 100-ID 재채점 — `db0b03c`

기준: [100항목 체크리스트](memory-state-substrate-review-checklist.md), 2026-09-24. 평가 소스는 clean commit `db0b03cd3e58a53dd5439a6578bb3b3d86eac593` (tree `388598df45c1f93a9ce4c8acc7b047fe43a30091`)이다. 감사 도중 HEAD가 문서·티켓만 바꾼 `83b1bd9`로 이동했으며 `db0b03c..83b1bd9`에는 Rust/Cargo/benchmark 코드 변경이 없다. 이 문서와 동시에 공유 작업트리에 생긴 다른 편집은 점수에 넣지 않았다. 점수는 확률이나 출하 허가가 아니라 [기존 rubric](memory-state-substrate-scored-review-2026-09-23.md#채점-기준)에 따른 계약·증거 강도다. 100은 관련 계약, 독립 음성 oracle, 동일 소스의 운영·host 증거까지 요구한다.

이후 `65f57d5`와 문서 정리 commit은 guidance/RFC 및 Cargo/Rust 주석만 바꾸어 실행 동작은 바꾸지 않았다. 이 후속 소스에 대해 점수를 다시 산정하지 않았다. 이 파일은 계속 `db0b03c` snapshot의 점수 기록이며, qualification receipt를 대신하지 않는다.

## 계산과 적용 범위

[최초 100행 점수](memory-state-substrate-scored-review-2026-09-23.md#항목별-결과)에 [clean `edfef66`의 40행 교체](memory-state-substrate-score-reaudit-2026-09-23-edfef66.md#확정-점수-상승-id)를 먼저 적용하고, 아래 12행을 다시 교체한다. 그 밖의 ID는 최초 또는 `edfef66` 점수를 **보수적으로 유지**한다. 따라서 세 표로 100개 ID 전부의 숫자가 결정되며 빈 점수는 없다. 이전 평가의 `범위외` 12개는 계속 80점의 범위 선언 점수를 기록하되 평균에서 제외한다. 변경되지 않은 점수는 새로 90점대의 증거를 찾았다는 주장이 아니다.

| 분모 | 적용 / 범위외 | 적용 점수 합계 | 평균 | 의미 |
| --- | ---: | ---: | ---: | --- |
| QMP가 제공하는 전체 capability | 88 / 12 | 5,917 / 8,800 | **67.24/100** | opt-in Redb까지 포함한 기존 점수와의 보수적 비교. Redb의 후속 코드 전체를 새로 독립 qualification한 점수가 아니다. |
| 현재 host 설정의 in-memory QMP capability 범위 | 81 / 19 | 5,456 / 8,100 | **67.36/100** | 위 점수에서 I02·I03·I07·I08·I09·I10·M05를 N/A로 제외. 각각 retryable durable transfer, durable commit, unknown-outcome reconciliation, restart discovery, durable bytes, orphan GC, crash/reopen이 현재 host 경로에 없다. 제공되지만 아직 소비되지 않는 in-process spill 항목은 포함한다. N/A는 구현의 품질 점수가 아니다. |

현재 host manifest는 facade `full`을 쓰지만 `spill-redb`는 켜지 않는다. host의 QMP API 호출에서 `RoutedSpillController`, `RetainingSpillTarget`, `InProcessSpillController`, `RedbSpillBacking` 소비도 확인되지 않았다. host의 production `ByteSized` 구현은 고정 해시 key, 소유한 `String` key의 길이, `Vec<u8>` payload 길이를 읽는다. 이 범위에서 내부 가변/shared sizing 결함은 재현되지 않았다. 다만 facade `full` 자체는 in-process spill helper를 제공하므로 I01·I04·I05·I06은 라이브러리 capability 점수에서 **계속 적용**한다. in-memory cache 유실은 허용되며 Redb의 물리 power-loss 자격을 이 출하 gate에 재도입하지 않는다. 향후 durable consumer가 생기면 제외한 7개를 즉시 적용으로 되돌린다.

## 이번에 바뀐 12개 ID

| ID | `edfef66` → 현재 | 관찰한 근거와 상한 |
| --- | ---: | --- |
| A05 | 42 → 58 | `facade/src/shared/measure.rs`와 `README-detailed.md`가 내부 가변/shared payload의 보수적 상한 또는 명시적 recharge/replace 책임을 정한다. 실제 host custom `ByteSized` 값의 독립 lifetime oracle은 없다. |
| B06 | 45 → 60 | 같은 비용 공식·원금 환급 계약이 명시됐다. 외부 타입의 사후 변경을 런타임에서 강제하지는 못한다. |
| E02 | 36 → 55 | payload/retained-map/transcript의 알려진 caller trait panic을 guard 밖에서 전파하거나 public 재사용 가능하게 했다. 일부 동기 caller 작업은 여전히 lock 안에서 실행되어 느린 호출 시간 상한은 없다. |
| E05 | 38 → 68 | `reference_eq_panic_before_mutation_keeps_public_store_available`, `checked_reads_propagate_client_clone_panics_without_poisoning_owner_mutex_v1`, transcript read-planning panic tests가 추가됐다. 모든 `Hash/Eq/Clone/Drop`·abort 조합을 증명하지는 않는다. |
| F06 | 35 → 78 | core의 same-broker 및 cross-thread reclaimer single-flight, event sink 재진입 음성 테스트가 통과했다. 비협조적 외부 callback 자체를 강제 중단하는 계약은 없다. |
| F07 | 48 → 73 | live-ticket count 포화에서 한 번만 회수하고 원장을 재측정하는 두 테스트가 통과했다. 일반 victim scan·I/O·host callback 총 실행시간 상한은 별개다. |
| L03 | 56 → 61 | v3 benchmark가 operation과 성공/거절 수, busy retry, throughput을 분리하고 검증한다. host의 성공·busy·reclaim·spill별 production tail SLO는 없다. |
| L04 | 36 → 67 | event sink single-flight와 10k reconfigure provenance 한도. 다른 metric cardinality 및 외부 sink 실행시간까지 상한을 증명한 것은 아니다. |
| M04 | 59 → 70 | payload Eq/Clone, retained-map Clone, transcript Ord/CanonicalKey, reclaimer panic의 public 후속 사용 음성 테스트. 열거된 모든 trait/adapter/allocator fault matrix는 아니다. |
| M06 | 70 → 76 | 220-cell full run은 `7f1c2f8`에서 생성됐고 현재 verifier는 성공·거절 count까지 fail-closed 검사한다. `db0b03c` smoke 48-cell 및 Python 20/20 GREEN. 동일 HEAD의 새 220-cell run, 운영 SLO, 호환 baseline 회귀 판정은 없다. |
| M09 | 30 → 62 | committed QMP `7f1c2f8` / Semantica `a71e3e9`의 두 host QBC rail은 GREEN이고 `7f1c2f8..db0b03c`에 Rust/Cargo 변경은 없다. 그러나 exact `db0b03c` host tuple, 현재 Semantica HEAD, 취소·메모리 peak·성능·activation은 미검증이다. |
| M10 | 65 → 72 | 이전 clean candidate에 canonical, extraction, raw 48/220 benchmark와 host tuple receipt가 있고 이번 HEAD의 verifier 20/20 및 focused core/facade/Cross-store GREEN을 확인했다. 이번 HEAD의 canonical GREEN은 명령·exit·전후 source fingerprint가 이 문서나 추적 receipt에 남아 있지 않은 관찰이므로 고정 소스 qualification 근거로 계산하지 않는다. 현재 HEAD의 새 full benchmark·host pair와 문서 수정까지 포함한 receipt도 없다. |

이 12개 변화의 순증은 +240점이다. 이전 clean `edfef66`의 5,677점에 더해 5,917점이다. 기존 12개 N/A 외에 in-memory에서 제외한 7개 ID의 이전 합계는 461점이므로 5,456/8,100이다. 소수점 1–2점 차이를 품질 차이로 해석하지 않는다.

## 이번 턴에 실제 확인한 증거

- `cargo test --locked -p quanta-memory-platform-core --all-features live_ticket_count_cap --quiet`: 선택 2/2 GREEN.
- `cargo test --locked -p quanta-memory-platform-core --all-features reentry --quiet`: 선택 3/3 GREEN.
- `cargo test --locked -p quanta-memory-platform-facade --all-features keeps_public_store_available --quiet`: 선택 5/5 GREEN.
- `cargo test --locked -p quanta-memory-platform-facade --features core-live-accounting,retained_map,revocable_store --test cross_store_hellgate --quiet`: 2/2 GREEN.
- `python3 -m unittest -q scripts/tests/test_scaling_results.py`: 20/20 GREEN.
- 기존 [source-bound receipt](../plans/sep-23-followup-misc/RECEIPT.md)의 220-cell benchmark 및 host pair는 `7f1c2f8` 근거다. `db0b03c`가 Rust/Cargo를 바꾸지 않은 사실은 재사용 판단에 도움을 주지만 **동일 SHA의 새 실행**을 대신하지 않는다. 이 점수 재감사는 Linux L2, physical power loss, hosted CI, 배포, activation을 수행하지 않았다.

## 100점까지 남은 판정, 우선순위

1. **P1 — 고정 소스 출하 증거:** 최신 QMP/host pair의 exact dependency identity와 실제 결과 정합성·취소·peak 측정, QMP canonical, clean full benchmark 220 cells, raw receipt를 같은 후보에 묶는다. 현재 shared dirty 문서를 통과한 과거 rail과 합쳐서 출하 완료로 쓰지 않는다.
2. **P2 — 현재 소비 경로의 미입증 계약:** D06/D08의 owner 종료·pin/pending 순서, F01/F03/H01의 host victim/recomputable 분류를 소스와 E2E로 대조한다. 결함이 재현되면 owner-local 수정과 독립 음성 oracle을 추가한다. A05/B06의 현재 production host 타입에서는 새로운 결함을 찾지 못했다. 점수 미달을 사고로 단정하지 않는다.
3. **P2 — 성능 기준을 먼저 정한 뒤 gate 강화:** 현재 verifier는 기본 p99 1초, throughput 90% 회귀 허용, 10ms 미만 baseline의 throughput 비교 생략이다. 이는 matrix/계측 무결성 guard이지 production SLO가 아니다. 대표 workload별 성공·busy·deny·reclaim tail과 허용 회귀폭을 host 요구에서 정하고, 안정적인 반복·호환 baseline을 확보한 뒤에만 release gate로 승격한다. 임의 threshold를 지금 코드에 박지 않는다.
4. **P2 — 제공하지만 host 미사용인 라이브러리 capability:** in-process spill의 retained handle/metadata 수명(I01/I04/I05/I06), policy pressure 진동(G03), 진단·long-run phase change(L05/L06/M07), generic callback/trait fault(E02/M04)를 별도 owner-local oracle로 닫는다. 실제 신규 consumer가 생기기 전 optional Redb를 active in-memory release gate로 되돌리지 않는다.

**판정:** 현재 확인한 P0은 없고 새 P1 구현 결함도 이 턴에 재현되지 않았다. 67점대를 100으로 올리려고 점수만 변경하거나 범위외를 확장하지 않는다. 특히 `M09/M10`은 host·receipt 권한이 다른 경계이므로 QMP unit GREEN으로 100을 줄 수 없다.
