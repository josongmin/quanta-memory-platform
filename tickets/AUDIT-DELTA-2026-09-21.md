# Current-source adversarial audit delta — 2026-09-21

This is source-bound evidence supplementing [`AUDIT-BASELINE.md`](AUDIT-BASELINE.md), not a second ticket inventory. Status, owner, dependencies and acceptance are authoritative only in [`README.md`](README.md) and each ticket. The frozen baseline at `e103771670da3143e2ffebb8950a5dfc7e3d4515` remains historical; this document never silently upgrades its GREEN or repeats its failures as current facts after repairs.

## Source and proof boundary

```text
product branch: main
committed product HEAD audited: 59f0c4fe46efc59dc371443b4d18acf38f1daf4b
QMP-120 integration: 4169a20 + c03b16c + 47021d3 (cherry-picked from independent proof branch)
QMP-050 and QMP-040 destructive leaves: committed through b18e47d;
  b18e47d clean-worktree doctor passed; new exact receipts and terminal gates pending
QMP-020 overflow: isolated RED on old source; fixed branch 4533146 integrated as 59f0c4f;
  isolated core GREEN and 59f0c4f clean product doctor PASS
foreign dirty excluded: root/member Cargo.toml, tools/ci/gates/gate-memory-platform-lint.sh,
  tools/ci/check_workspace_lints.py, tools/ci/tests/
proof: read-only reachability/source review + isolated QMP-020 and QMP-040 RED;
  b18e47d and 59f0c4f clean doctor PASS; post-integration 2a827af local
  actual extraction/clean git PASS; no later ticket-doc SHA extraction,
  remote CI, host QBC, performance benchmark or production RSS proof
```

The purpose remains portable, host-independent budgeting/accounting with a single fail-closed charge owner, `core <- facade <- testkit`, and reproducible standalone extraction. Dependency direction and source-layer purity survived the baseline. The product is **not qualified or production-ready**: QMP-020 overflow and QMP-040 tested destructive leaves have clean integrated product proof, but QMP-040 callback-panic policy, bounded transcript evidence, spill transfer, independent conformance and external host adoption remain open. QMP-120 actual extraction at post-integration `2a827af` passed locally; remote workflow proof and exact later ticket-doc SHA/host adoption are absent.

## Delta findings — distinguish executed RED from static reachability

| ID / severity / proof | Source and reachable failure | Authority / ticket |
| --- | --- | --- |
| P1-D01 / historical RED, integrated fix doctor GREEN | At `f1160c9`, `core/src/domain/ledger.rs:344,359-371` saturated prospective sums and issued an unaccounted live ticket after `MAX-10` then `100`; scratch reserve/recharge also admitted overflow. Isolated `4533146` replaced admission/release/scheduler arithmetic with checked preflight; main `59f0c4f` clean doctor passed. | [QMP-020](QMP-020-reservation-lifecycle/TICKET.md); no saturation is a legal admission authority. |
| P1-D02 / historical RED, b18 clean doctor GREEN | `eb58374` payload delete raw-dropped a generic entry and retained-map remove raw-dropped a generic key after refund. Two child subprocesses aborted with signal 6 from paired destructor panics. At `b18e47d`, both leaves retire fields separately outside locks and owner subprocess tests pass. **Outer caller unwind on these exact leaves remains untested.** | [QMP-040](QMP-040-store-lifecycle/TICKET.md); preserve exact postimage, then separately contain each destructor. |
| P2-D01 / STATIC | Payload dedup/new put, transcript replay/load, retained-map get still call client traits while holding the state mutex. A single client callback panic poisons it; subsequent operations return `InvalidState` via `shared/lifecycle.rs:69-79`. This is an availability/contract gap, **not** evidence of a ghost entry or ledger drift. | [QMP-040](QMP-040-store-lifecycle/TICKET.md); choose and prove a supported-client panic policy. |
| Exception-boundary candidate / STATIC | `shared/lifecycle.rs:22-24` discards the `catch_unwind` `Err` payload. Rust documents that dropping a panic payload may panic; an already-unwinding caller can abort. Existing owner child fixtures use string payloads, not a panicking payload destructor. | [QMP-040](QMP-040-store-lifecycle/TICKET.md); subprocess RED/contract decision, without claiming `catch_unwind` covers abort-mode panics. |
| P1-09 refined / STATIC | `facade/src/revocable_store/service.rs:258-284` refunds source before returning value; `spill/service.rs:81-99` consumes the handle even on unavailable/accept error. No caller can recover the removed source after that error. In-memory target preserves RSS with no destination charge; Noop reports successful discard. | [QMP-070](QMP-070-spill-transfer-backings/TICKET.md); typed source/destination transfer receipt, recoverable error ownership. |
| P2-07 refined / STATIC | `testkit/src/conformance/target.rs:232-245` maps observation failure to zero; `suites.rs:355-405` accepts self-reported balance and ignores commit/finish/release results. Black-hole accounting can appear balanced. | [QMP-090](QMP-090-conformance-model/TICKET.md); independent expected-state machine and broken-fake RED. |
| P2-03 corrected / source census | `record_group_usage` from the baseline is absent in current `core/src`; do not claim it remains public or reimplement its removal. Other QMP-080 defects—unused soft/reserved/burst knobs, borrowing reconfigure mismatch, unwired reclaim, stale provenance—remain source-reachable. | [QMP-080](QMP-080-policy-runtime-semantics/TICKET.md); public API absence guard and remaining field-to-consumer tests. |
| P2 candidate / STATIC, semantic decision pending | Pre-fix `domain/ledger.rs:344-350` counts Headroom usage toward the non-Headroom ceiling at reserve, while `api/control.rs:867-877` compares only non-Headroom usage at reconfigure. Re-audit current `59f0c4f` predicates after the QMP-020 rewrite before treating it as current failure. | [QMP-080](QMP-080-policy-runtime-semantics/TICKET.md); decide protected-band semantics, then RED and shared predicate. Not yet a confirmed behavioral failure. |
| P2-12 extended / STATIC | At `59f0c4f`, ledger single release/batch/rollback clones complete accounting maps and traverses the full snapshot for validation under the broker lock (`ledger.rs:591,663-684`); broker also clones scheduler (`api/control.rs:624-643`). Reserve already had full-map work. With N resident owners, N sequential releases can be O(N²); no 10k/100k measured latency yet. | [QMP-100](QMP-100-scaling-benchmarks/TICKET.md); affected-slot checked preflight/aggregate plus frozen cardinality/latency benchmark, without weakening exactness. |

No P0 claim is made. Historical RED remains evidence of the bug, not a claim that the same bug persists after a fix. Original P1/P2 IDs and counts are frozen; new IDs are additive. The QMP-020 scheduler public `record_usage`/`record_release` return type changed to `MemoryResult<u64>`; public `LedgerSnapshot::total_*` now panics on an externally forged unrepresentable snapshot while broker observation validates first. QMP-110/140 own typed observation/SemVer policy, and QMP-130 must compile the external adapter. No valid-broker panic is alleged.

## Implementation order and ownership barriers

1. **Core exactness, QMP-020**: `59f0c4f` integrates checked all-axis ledger/scheduler admission, rollback and scratch/recharge repair with isolated RED→GREEN and clean integrated doctor. A later clean product `2a827af` actual extraction also passed. Finish external host receipt, classify public scheduler signature change, and preserve overflow/denial preimage on any later hot-path optimization. The previous reservation typestate receipt alone cannot qualify overflow.
2. **Store destructive leaves, QMP-040**: `b18e47d` already repairs the two tested raw-drop leaves and a clean doctor passes. Add the still-missing outer caller unwind child fixture on those exact leaves, then decide callback-panic support versus an explicit client type contract; Busy on reentry is not poison recovery. Keep P2-D01 visible until a supported-client policy and negative gate exist.
3. **Retained-cost closure, QMP-050**: `b18e47d` implements semantic-cost accounting and independent prepared/transcript underbilling fakes with clean doctor. Extend generic lifecycle/oracle coverage only where evidence is missing, distinguish honest-client reported bytes from physical RSS, and require QMP-020/060/090/120/130 separate terminal proof. Do not promote this scoped receipt to `QUALIFIED`.
4. **Bounded identity and transfer, QMP-060/070 (`READY`)**: QMP-050 formula is frozen for owner work. Choose deterministic transcript idempotency horizon, then implement spill source/destination receipt with recoverable errors; no successful Noop persistence. Validate redb reopening and advertised backing parity or remove unsupported feature claims. Coordinate revocable/transcript writes with remaining QMP-040 callback policy work.
5. **Policy and oracle, QMP-080 (`READY`)/090 (`BLOCKED`)**: policy fields need reachable reserve/reclaim/reconfigure consumers and same prospective predicate. The conformance suite needs a fallible observation and independent state trace; every broken fake must fail at the causal step. Run concurrency model/negative fixtures separately from ordinary stress tests.
6. **Scale, operation, extraction, host, release, QMP-100/110/120/130/140**: benchmark O(N²) cuts with frozen workload and contention quantiles, wire bounded telemetry without making it accounting authority, observe QMP-120 remote CI and distinguish the passed `2a827af` local extraction from any later exact source, then qualify Semantica adapter in its own exact-head lane. Only then reconcile public SemVer and documentation claims.

Parallel work is safe only on disjoint owner files. Ledger QMP-020 and policy QMP-080 both touch core admission; serialize their final integration. Transcript QMP-050 then QMP-060, and revocable QMP-050 then QMP-070, are serial ownership boundaries. The external host may inventory call sites meanwhile, but cannot use a moving QMP-050 public API as final QMP-130 qualification.

## Qualification matrix at this audit

| Layer | Observed | Not established |
| --- | --- | --- |
| Ticket governance | `scripts/verify-tickets.sh` 7 validator fixtures and current edited tree pass after b18/59 integration | Final SHA terminal validation and remaining ticket receipts. |
| Core | QMP-020 isolated `4533146` four RED→GREEN and core all-features 177+7+14+11 pass; integrated `59f0c4f` clean doctor passes | Final-product extraction and external scheduler consumer migration, QMP-080 policy semantics. |
| Facade | `b18e47d` clean doctor passes (including facade 233 unit, 18 hellgate, 21 public, 17 doc; testkit 58+4+1), with QMP-050 semantic cost and QMP-040 destructive-leaf tests | Exact-leaf outer unwind, callback policy, general QMP-090 state oracle, QMP-060/070. |
| Extraction | QMP-120 isolated `e6f69c3` offline frozen rail; integrated post-QMP-020/050 `2a827af` fresh actual extraction, canonical digest `effac781...`, check-workspace/doctor, clean extracted git | Remote workflow, integrated-source offline `--frozen`, later ticket-doc SHA/host adoption. |
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
