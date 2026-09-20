# Current-source adversarial audit delta — 2026-09-21

This is source-bound evidence supplementing [`AUDIT-BASELINE.md`](AUDIT-BASELINE.md), not a second ticket inventory. Status, owner, dependencies and acceptance are authoritative only in [`README.md`](README.md) and each ticket. The frozen baseline at `e103771670da3143e2ffebb8950a5dfc7e3d4515` remains historical; this document never silently upgrades its GREEN or repeats its failures as current facts after repairs.

## Source and proof boundary

```text
product branch: main
committed HEAD audited: 47021d3271c3613c861ad4c06c4d3c2b4b1edb17
QMP-120 integration: 4169a20 + c03b16c + 47021d3 (cherry-picked from independent proof branch)
QMP-050: IN_PROGRESS; shared/builder, payload, retained-map, transcript,
  prepared leaf and testkit changes not yet frozen as a product commit
foreign dirty excluded: root/member Cargo.toml, tools/ci/gates/gate-memory-platform-lint.sh,
  tools/ci/check_workspace_lints.py, tools/ci/tests/
proof: read-only reachability/source review + isolated QMP-020 RED + focused
  transcript testkit selector + historical exact-SHA receipts; no final
  integrated workspace, remote CI, host QBC or production RSS proof
```

The purpose remains portable, host-independent budgeting/accounting with a single fail-closed charge owner, `core <- facade <- testkit`, and reproducible standalone extraction. Dependency direction and source-layer purity survived the baseline. The product is **not qualified or production-ready**: even though several P1 baseline slices have implementation receipts, QMP-020 has a newly executed overflow RED; QMP-040 has source-derived destructive panic gaps; retained-cost, spill transfer, independent conformance and external host adoption remain open. QMP-120's local extraction repair was integrated at this HEAD and has an isolated branch receipt, but remote workflow proof and an exact final-product extraction are still absent.

## Delta findings — distinguish executed RED from static reachability

| ID / severity / proof | Source and reachable failure | Authority / ticket |
| --- | --- | --- |
| P1-D01 / RED | `core/src/domain/ledger.rs:344,359-371` saturates prospective sums, `:156-159` saturates counters, and `:997-1014` blesses the behavior. Under a validated `u64::MAX` policy, reserve `MAX-10`, then `100`: second live ticket is wrongly admitted and the post-release balance loses bytes. Isolated core owner tests failed exactly on these assertions. `api/control.rs:561,714` scratch recharge/reserve also admitted beyond `MAX` in isolated RED. | [QMP-020](QMP-020-reservation-lifecycle/TICKET.md), status re-opened `READY`; core ledger is the producer. |
| P1-D02 / STATIC, subprocess RED pending | `facade/src/payload_store/service.rs:340-347` raw-drops a whole generic entry after refund; `retained_map/service.rs:516-522` raw-drops generic key before returning value. Two panicking field destructors (or an existing unwind) can abort. The canonical per-field `retire_client_value` already exists but is bypassed by these leaves. | [QMP-040](QMP-040-store-lifecycle/TICKET.md); exact refund/postimage then separately contain each client destructor. |
| P2-D01 / STATIC | Payload dedup/new put, transcript replay/load, retained-map get still call client traits while holding the state mutex. A single client callback panic poisons it; subsequent operations return `InvalidState` via `shared/lifecycle.rs:69-79`. This is an availability/contract gap, **not** evidence of a ghost entry or ledger drift. | [QMP-040](QMP-040-store-lifecycle/TICKET.md); choose and prove a supported-client panic policy. |
| P1-09 refined / STATIC | `facade/src/revocable_store/service.rs:258-284` refunds source before returning value; `spill/service.rs:81-99` consumes the handle even on unavailable/accept error. No caller can recover the removed source after that error. In-memory target preserves RSS with no destination charge; Noop reports successful discard. | [QMP-070](QMP-070-spill-transfer-backings/TICKET.md); typed source/destination transfer receipt, recoverable error ownership. |
| P2-07 refined / STATIC | `testkit/src/conformance/target.rs:232-245` maps observation failure to zero; `suites.rs:355-405` accepts self-reported balance and ignores commit/finish/release results. Black-hole accounting can appear balanced. | [QMP-090](QMP-090-conformance-model/TICKET.md); independent expected-state machine and broken-fake RED. |
| P2-03 corrected / source census | `record_group_usage` from the baseline is absent in current `core/src`; do not claim it remains public or reimplement its removal. Other QMP-080 defects—unused soft/reserved/burst knobs, borrowing reconfigure mismatch, unwired reclaim, stale provenance—remain source-reachable. | [QMP-080](QMP-080-policy-runtime-semantics/TICKET.md); public API absence guard and remaining field-to-consumer tests. |
| P2 candidate / STATIC, semantic decision pending | `domain/ledger.rs:344-350` counts Headroom usage toward the non-Headroom ceiling at reserve, while `api/control.rs:867-877` compares only non-Headroom usage at reconfigure. Same live state can receive contradictory legal-state judgments. | [QMP-080](QMP-080-policy-runtime-semantics/TICKET.md); decide protected-band semantics, then RED and shared predicate. Not yet a confirmed behavioral failure. |

No P0 claim is made. The P1-D02 mechanism follows Rust's destructor/unwind behavior, but its owner-specific subprocess test has not yet been executed. Original P1/P2 IDs and counts are frozen; new IDs are additive.

## Implementation order and ownership barriers

1. **Core exactness, QMP-020**: write RED tests at ledger/broker owner on all admission axes and recharge, then use checked prospective totals before any ticket or scheduler mutation. Replace the saturation-acceptance test, prove denied preimage, final drain, and honest `u64::MAX` boundary behavior. Coordinate QMP-080 scheduler saturation; never mask only a facade consumer. The previous reservation typestate receipt is preserved but cannot qualify overflow.
2. **Store destructive leaves, QMP-040**: prove payload delete and retained-map remove in child processes with multiple panicking generic fields and an outer unwind; route every postimage destruction through per-field retirement outside locks. Separately decide callback-panic support versus an explicit client type contract; Busy on reentry is not poison recovery. Freeze new owner tests before re-issuing a lifecycle receipt.
3. **Retained-cost closure, QMP-050**: integrate current disjoint leaf work, run independent testkit literals and denial/replay/truncate/abort/GC equality. Explicitly distinguish semantic reported bytes from physical RSS and malicious `ByteSized` lies. Recheck generic callback panic/preimage after adding measurement. Do not mark `QUALIFIED` from a dirty worktree or one selector.
4. **Bounded identity and transfer, QMP-060/070**: after cost ownership is stable, choose deterministic transcript idempotency horizon, then implement spill source/destination receipt with recoverable errors; no successful Noop persistence. Validate redb reopening and advertised backing parity or remove unsupported feature claims.
5. **Policy and oracle, QMP-080/090**: policy fields need reachable reserve/reclaim/reconfigure consumers and same prospective predicate. The conformance suite needs a fallible observation and independent state trace; every broken fake must fail at the causal step. Run concurrency model/negative fixtures separately from ordinary stress tests.
6. **Scale, operation, extraction, host, release, QMP-100/110/120/130/140**: benchmark O(N²) cuts with frozen workload and contention quantiles, wire bounded telemetry without making it accounting authority, observe QMP-120 remote CI and repeat actual extraction at final product SHA, then qualify Semantica adapter in its own exact-head lane. Only then reconcile public SemVer and documentation claims.

Parallel work is safe only on disjoint owner files. Ledger QMP-020 and policy QMP-080 both touch core admission; serialize their final integration. Transcript QMP-050 then QMP-060, and revocable QMP-050 then QMP-070, are serial ownership boundaries. The external host may inventory call sites meanwhile, but cannot use a moving QMP-050 public API as final QMP-130 qualification.

## Qualification matrix at this audit

| Layer | Observed | Not established |
| --- | --- | --- |
| Ticket governance | `scripts/verify-tickets.sh` 7 negative/positive fixtures and current tree passed at `47021d3` plus ticket delta | New findings' owner RED→GREEN receipts and final SHA terminal check. |
| Core | QMP-010/020/030 have historical focused implementation receipts; QMP-020 overflow and two scratch operations are newly RED | Core exactness after repair, all-features and workspace final source, host behavior. |
| Facade | QMP-040 clean `eb58374` doctor/feature/negative-fake receipt; QMP-050 leaves in progress; transcript testkit honest selector passed on dirty integration | New destructor/callback gates, prepared leaf integration, all stores exact model, QMP-060/070. |
| Extraction | QMP-120 isolated `e6f69c3` branch fresh actual extraction, canonical digest, clean extracted git and frozen offline rail; integrated content at `47021d3` | Remote workflow result, final QMP product exact SHA extraction/host adoption. |
| SOTA comparison | [`REFERENCES.md`](REFERENCES.md) maps official DataFusion reservation ownership, kernel protection/pressure/hard controls, Rust/Cargo semantics and observability requirements to owner tickets | External design references are not runtime proof; a benchmark or model check passes only its recorded scope. |

### Re-audit commands after each product freeze

```bash
git rev-parse HEAD
git status --short
scripts/verify-tickets.sh
python3 tools/pm/pm.py lint
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
scripts/check-workspace.sh
scripts/memory-platform-doctor
cargo deny --all-features check
# Then fresh actual extraction to a unique empty temp destination, followed by
# extracted clean git and locked/frozen rails; never reuse a prior green SHA.
```

Run host QBC and the performance workload in separate lanes with their own source identities. A local focused test, static audit, or isolated old-source receipt is not final qualification of this moving repository.
