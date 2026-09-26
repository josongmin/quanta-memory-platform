# SMA-009: Semantica product qualification

Status: Open until an exact-source host receipt demonstrates every applicable
gate below. The earlier integration of SMA-001/002/003 is implementation,
not final product proof. This ticket lives in QMP for dependency and contract
visibility; Semantica owns execution and its source-bound receipt.

## Freeze before execution

Record both repository HEADs and dirty fingerprints, both Semantica workspace
manifest pins, the Cargo-resolved QMP source/features, selected QBC
owner/profile, nonzero test counts, exit status and artifact paths. Revalidate
the current host checkout before using any earlier source-specific failure or
success. Do not merge an independently dirty candidate merely to satisfy this
ticket.

## Acceptance matrix

| Gate | Required result |
| --- | --- |
| G1 API cutover | Removed handle overlay/selection API has no supported caller; actual public SDK consumer compiles. |
| G2 Defaults and observation | Checked defaults/overflow and global/headroom/class policy survive; report/stats project the installed broker coherently. |
| G3 Product policy | Existing SDK artifact reservation and default-share contract remain unchanged. |
| G4 Admission bridge | Real publication receives typed denial at the installed cap, preserves store/ledger postimage, and succeeds after refund. |
| G5 Memo custody | Memo row/index cost and receipt/prepared/GC/cancel/retract/close leases remain on the installed broker through final ownership. No charge is duplicated or refunded early. |
| G6 SDK query | Public query miss→publish→hit→invalidate/GC, cap denial→typed error→subsequent success, then close/drain execute against a broker-backed memo table. |
| G7 Final source | G1–G6 and G8 apply to the same final integration candidate; feature and dependency resolution match that candidate. |
| G8 SQL worker | CLI feature closure and reachable rulepack-worker process path execute. Each process installs its own bounded domain; paired legs prove combined admission, denial, release and retry. An unused CLI helper or a composer-only test is not worker product proof. |

Reuse registered Semantica owner/QBC tests first. Owner-local invariants and
cross-owner product transitions are separate receipts; a selector resolving,
`rustfmt` passing, or a test source existing is not a GREEN execution. A
resource preflight failure is an environment block, not a test result. Run
new end-to-end tests only for transitions existing owner suites cannot prove.

Prior observations were snapshot-specific: composer owner tests passed on an
earlier Semantica source, while later CLI/worker attempts were blocked or RED
under different sources. The historical details remain in Git history, not in
the current verdict. Physical RSS, Redb power loss and benchmark superiority
are not acceptance claims here. Integration decisions are in
[ADR 0004](../../adr/0004-host-adoption-boundary.md).
