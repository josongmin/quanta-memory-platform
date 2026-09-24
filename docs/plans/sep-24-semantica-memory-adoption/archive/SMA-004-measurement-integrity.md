# SMA-004 — 제품 메모리 benchmark의 측정·실행 증거

ARCHIVED DESIGN — 실행 티켓 아님. 알려진 metric 결함은 보존하되 현재 예산 API 수정과 분리한다.

Priority: P2 measurement correctness. Status: BACKLOG — 활성 예산 수정 목록 제외.
해당 metric을 사용하는 성능 작업에서만 시작한다. 001/002의 correctness gate가 아니다.
Owner: Semantica benchmark metrics/probe. Depends on: snapshot. Evidence: [E07](../AUDIT.md).

## 파일과 범위

- `C/quanta-bench/src/process_metrics.rs`
- `C/quanta-bench/src/isolated_runner.rs`와 `isolated_runner/{evaluation,report}.rs`는
  **005가 이 runner를 채택할 경우에만** 수정 대상으로 승격.
- `C/quanta-runtime/src/env_scope_guard.rs` 및 testing helper의 기존 getrusage 구현은 재사용 후보.
- 새 product probe는 기존 `quanta-bench/src/bin/analysis_hotpath_probe.rs` 또는 실제
  runtime-search/daemon benchmark owner에 등록한다. 적용 target/features를 먼저 결정한다.

## 작업

1. 005가 실행할 binary/child entrypoint/metric producer를 명시한다.
   미사용 generic runner를 광범위하게 고치는 작업으로 확장하지 않는다.
2. macOS peak는 high-water mark를 사용하고 byte 단위를 검사한다.
   기존 getrusage helper의 소유 계층을 확인해 하나를 재사용/통합한다. 새 probe마다
   동일 OS 변환기를 복제하지 않으며 production→test 역방향 의존을 만들지 않는다.
   Linux VmHWM/getrusage의 KiB와 macOS ru_maxrss bytes를 혼동하지 않는다.
   `current_rss_mb`와 peak API의 이름/의미를 분리하며 unavailable은 Option/typed status로 보존한다.
3. case마다 새 process로 peak를 초기화한다. 단일 process의 누적 peak를 case delta처럼
   빼서 계산하지 않는다. semantic bytes와 RSS를 같은 cap에 대입하지 않는다.
4. child protocol에는 case ID, source digest, actual scenario count, result digest, metric
   source/unit, start/end identity를 넣는다. parent가 이를 검증한다.
5. 선택한 runner는 timeout을 실제 child supervisor에 전달하고 초과 시 종료+reap한다.
   hard timeout claim에는 descendant process 정책도 정한다. stdout/stderr를 동시 drain하여
   pipe deadlock과 무제한 output 적재를 막는다.
6. spawn/parse/timeout/metric failure는 invalid/degraded evidence다.
   in-process fallback으로 실행 성공은 얻어도 isolated memory gate를 PASS로 만들지 않는다.
7. cold/warm, corpus digest, parallel load, feature graph, machine/toolchain, command,
   sample count를 raw artifact에 기록한다. baseline 비교는 호환 조건을 검증한다.

## 최소 테스트

- bytes/KiB conversion의 boundary/overflow/unavailable을 결정적 unit test로 검사.
- 무해한 child가 allocation→touch→release를 수행한 뒤 OS high-water를 기록하는 smoke.
  allocator 해제량이나 정확한 RSS delta를 고정 수치로 기대하지 않는다.
- 잘못된 case identity, missing metric, truncated/extra row, child failure가 proof에서 거절됨.
- pipe에서 대기하는 child의 bounded termination/reap; 잘못된 output parser가 fallback
  PASS로 바뀌지 않음. 운 좋은 sleep 간격에 따라 성공하는 테스트를 만들지 않는다.
- 기존 metric consumer가 영향을 받으면 해당 package의 좁은 regression만 추가.

## 검증/완료

실제 owner package/profile을 README 방식으로 resolve한다. test filter만으로 큰
quanta-bench compile closure가 작아졌다고 주장하지 않는다.
OS-specific smoke는 macOS/Linux 중 실행한 platform만 qualified로 기록한다.

완료: 005가 사용하는 binary와 metrics path가 실제로 검증되고 invalid evidence가
거절되는 receipt. generic runner가 unused이면 그 발견과 미수정 범위를 적고 닫을 수 있다.
