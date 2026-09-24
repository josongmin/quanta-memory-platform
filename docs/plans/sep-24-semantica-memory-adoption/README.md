# Semantica × QMP — 메모리 예산 채택 최종 실행 계획

감사일: 2026-09-24. 활성 범위는 **001/002/003/009**다. 이 문서는 실행 목록의
SSOT이며, [이전 001/002 분리·통합 기록](INTEGRATION-2026-09-24.md)은 당시의
스냅샷 증거다. 이후 canonical memo 채택이 제품 범위에 들어왔으므로 해당 기록의
“memo 제외” 판정은 현재 계획에 적용하지 않는다.

SMA-001/002/003 구현은 Semantica main `31a07f43e1f09ab795d7e8832eb82273e334cd98`
(`17b2702` API + `31a07f4` memo)로 fast-forward 통합했다. 원본
`/Users/songmin/Documents/code-new/semantica-memory-adoption-codex`
(`80822f963760cfd33628c1085cc0ec2ceea2411f` + dirty diff)은 보존했다.
이전 `0d04d66` 부분 후보와 과거 GREEN을 최종-main proof로 합산하지 않는다.
통합 경과와 제외한 비메모리 hunk는 [통합 기록](INTEGRATION-2026-09-24.md)에 있다.

## 기준 소스와 기존 변경

- QMP: `a1179a10c5569619210cb1c03ceeae7430109c25`. 이번 변경은 문서뿐이다.
- Semantica main integration point: `31a07f43e1f09ab795d7e8832eb82273e334cd98`,
  shared dirty work remains outside the merged paths.
- 원본 candidate: `/Users/songmin/Documents/code-new/semantica-memory-adoption-codex`,
  detached HEAD `80822f963760cfd33628c1085cc0ec2ceea2411f` + 보존된 uncommitted changes.
- main에는 overlay/미집행 축 제거, canonical memo lease, typed capacity 전파가 있다.
  **main 통합 ≠ 최종 소스 제품 검증**.
- QBC owner lib/all-features: adapter 122/122, memory-host 119/119, composer 51/51.
  composer 51/51 이후 교차-owner memo 테스트와 Cargo dev-dependency가 추가됐으므로
  그 결과는 현재 최종 diff의 GREEN이 아니다. SDK/query 제품 E2E도 미실행이다.
  현재 내부 디스크 여유가 1 GiB 미만이고 QBC capacity gate는 최소 50 GiB 및
  4.75% 여유를 요구한다. 저장공간 문제는 테스트 실패와 구별한다.
- 근거와 변경 fingerprint: [FINAL-AUDIT.md](FINAL-AUDIT.md). 이전 조사: [AUDIT.md](AUDIT.md).
- `S/`: Semantica root, `C/`: `S/packages/analysis/quanta-v2/crates/`, `Q/`: QMP root.
  candidate의 `C/`도 동일한 상대 경로다.

## 해야 할 작업만

| 티켓 | 우선순위 / 현재 상태 | 실제 남은 일 |
| --- | --- | --- |
| [SMA-001](SMA-001-budget-domain-api.md) | P1 / main 통합, 최종 source 미검증 | public SDK cutover·installed report/stats·default validation을 정확한 feature에서 검증 |
| [SMA-002](SMA-002-execution-axis-contract.md) | P1 / main 통합, 최종 source 미검증 | 미집행 output/scratch API 제거, 기존 gateway rail 보존을 검증 |
| [SMA-003](SMA-003-canonical-memo-accounting.md) | P1 / main 통합, raw owner-local 부분 GREEN, 제품 증거 없음 | QBC/all-features 및 row 비용·lease 수명·typed 실패와 실제 SDK ingress 연결 검증 |
| [SMA-009](SMA-009-product-qualification.md) | P1 / 제품 검증 미완료 | 고정된 QMP–Semantica main 쌍의 owner/public/product E2E 및 최종 소스 재검증 |

001/002는 같은 API diff를 공유하므로 한 owner가 순차 인수한다. 003의
ports/adapter/host 비용·수명 검증은 병행할 수 있지만 Cargo.lock, composer,
SDK ingress, runtime error 경계는 직렬 integrator가 소유한다. 현 증거로 P0는
단정하지 않는다. P1은 제품 계약·과금 수명·출하 증거의 우선순위다.

## 작업에서 제외한 것

| 항목 | 제외 이유 |
| --- | --- |
| 기본 live report, 단일 `observe()` stats | main에 이미 구현됨. 기존 회귀만 보존 |
| 실행 counter/permit, 기본 MiB overflow/manifest 검증 | 기존 구현 재사용. 새 엔진·resolver·동일 목적 suite 금지 |
| overlay/축 제거 코드의 신규 작성 | main에 통합됨. 최종 소스 검증만 남음 |
| owner-key 재정의/중복 제거 신규 작업 | main은 기존 host constants를 reexport. 별도 P1 작업 없음 |
| 별도 FIFO memo mirror·공통 reclaim | canonical row 수명에 붙은 lease와 별개이며 이번 제품 경로의 필요성이 입증되지 않음 |
| IDE 벤치·추가 cache·디스크 backend | 이번 예산 계약 수정의 선행 조건이 아님 |
| 새 거대 root E2E·dual-authority metering | owner-local 증명을 대체하지 않음. 필요한 cross-owner 제품 전이만 최소 E2E로 추가 |

SMA-003은 현재 범위로 재개했다. [archive의 이전 설계](archive/SMA-003-canonical-memo-accounting.md)는
근거 자료일 뿐 활성 완료 기준은 새 티켓이다. SMA-004~008은 계속 조건부/보류다.
macOS peak/RSS·성능 우월성은 이번 correctness receipt에서 주장하지 않는다.

## 실행 및 검증 규칙

1. main의 변경 범위, HEAD/dirty diff/실제 Cargo dependency를 고정한다.
2. 001/002는 기존 테스트와 public consumer를 재사용한다. 003은 row-local oracle과
   실제 설치 broker의 ledger snapshot을 분리 검증한다. source census는 compile proof가 아니다.
3. Semantica는 등록 QBC owner/profile을 사용한다. 큰 runtime lib(test)나 root E2E를
   단일 assertion 때문에 새로 빌드하지 않는다. 신규 cold build는 한 번에 하나다.
4. 지원 feature에서 nonzero 테스트 실행 수, exit, source/diff digest를 기록한다.
   all-features 한 번으로 production/minimal feature proof를 대체하지 않는다.
5. 009에서 query→memo publish/hit/invalidate, cap+1 실패 후 정상 요청, close·drain을
   실제 제품 경로로 실행한다. owner GREEN을 제품 GREEN으로 승격하지 않는다.
6. main 통합은 완료됐다. 현재 main의 dirty work는 별도 owner 소유이며,
   최종 출하 판정 전에 HEAD와 dirty fingerprint를 다시 고정한다.

QBC 조회 예시이며 placeholder는 실제 owner 결과로 치환한다:

```sh
scripts/quanta-build-cli owner resolve --package <owner-package> --target-kind lib --compile-policy package-all-features
scripts/quanta-build-cli profile run <registered-profile-id>
```

실제 target/filter는 기존 profile/owner가 반환한 명령을 사용한다. 등록이 없다는 이유로
새 catalog나 root test를 만들지 않는다. 기존 FIN-20/host-pair receipt는 source/features가
맞는 범위에서만 재사용하고 영향받은 경계만 재검증한다.

QMP runtime/API/Cargo를 바꾸지 않는 것이 현재 범위다. 따라서 QMP 신규 canonical rail,
추출 검증, Redb qualification을 이 작업의 선행 gate로 추가하지 않는다.
실제 QMP 변경이 필요해지면 범위를 다시 검토하고 기존 repository rail을 적용한다.

완료 상태는 `MAIN_INTEGRATED_UNQUALIFIED → OWNER_GREEN → PRODUCT_GREEN → FINAL_SOURCE_VERIFIED`.
테스트 미실행은 검증 잔여이지 이미 작성된 코드를 재구현할 이유가 아니다.
