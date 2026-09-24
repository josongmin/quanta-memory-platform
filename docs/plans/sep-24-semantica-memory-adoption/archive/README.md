# 제외한 설계 보관

이 디렉터리는 실행 티켓 목록이 아니다. [현재 목록](../README.md)의 001/002/003/009에
필요하지 않은 조건부 설계와 과거 검증 matrix를 복구 가능하게 보존한다.
기존 ID는 출처 추적용이며 구현 완료/승인/자동 재개를 의미하지 않는다.

| 과거 ID | 보관 내용 | 제외 이유 |
| --- | --- | --- |
| [SMA-003](SMA-003-canonical-memo-accounting.md) | 최초 memo charge/receipt 설계 | 현재 범위로 재개됨; [활성 티켓](../SMA-003-canonical-memo-accounting.md)이 우선 |
| [SMA-004](SMA-004-measurement-integrity.md) | peak RSS/child runner | 실제 metric 결함이지만 이번 API correctness와 별도 범위 |
| [SMA-005](SMA-005-ide-pressure-benchmark.md) | IDE pressure 벤치 | 현재 변경에서 성능 주장을 하지 않음 |
| [SMA-006](SMA-006-targeted-pressure-reclaim.md) | 공통 reclaim | 실제 회수 수요와 owner 선정이 선행 |
| [SMA-007](SMA-007-additional-cache-selection.md) | parser/PTA/trace 도입 | 현재 제품 비용/채택 근거 미확정 |
| [SMA-008](SMA-008-durable-cache-decision.md) | disk backend 비교 | named consumer/도입 결정 없음 |

이 파일들은 삭제하지 않았다. 향후 범위가 채택되면 당시 소스와 요구를 다시 확인하고
필요한 항목만 작업 목록으로 승격한다. 보관본의 명령/라인/상태는 역사적 설계다.
