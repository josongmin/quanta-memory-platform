# Current plan status (2026-09-24)

The `db0b03c` snapshot below is the last audited implementation base, not
current HEAD. Current local qualification is asserted only when an annotated
`qmp-local-qualified-*` tag peels to exact `HEAD`; its annotation binds the
canonical workspace rail, fresh full benchmark, and extraction dry-run. Check
the tag target before relying on a receipt after any new commit. Host-pair,
hosted CI, deployment, and activation are separate claims.

| Plan | Implementation | Evidence and remaining gate |
| --- | --- | --- |
| [Sep-23 FUP-001/002](sep-23-followup-misc/README.md) | Implemented and historically verified for opt-in Redb; outside the current in-memory product scope. | Prior Redb receipts remain historical. A future durable consumer needs its own feature and failure-model qualification. |
| [Sep-23 FUP-003/004/005/007](sep-23-followup-misc/README.md) | Implemented with owner-local GREEN and temporary-mutant RED. | The Sep-23 candidate has an annotated v5 receipt. The newer in-memory pair was qualified at QMP `7f1c2f8` with Semantica `a71e3e9`; neither receipt names this audit base. |
| [Sep-23 FUP-006](sep-23-followup-misc/FUP-006-integration-qualification.md) | Prior local QMP and committed-host pair gates passed. | Verifier change and focused proof are at `db0b03c`; current local qualification requires an exact-target `qmp-local-qualified-*` receipt. Extraction dry-run is included; host proof is separate and conditional on a host-adoption claim. |
| [Sep-24 T1](sep-24-adversarial-e2e-bench/README.md) | Cross-store E2E implemented. | Two selected cases passed at the `db0b03c` audit base; not rerun at current HEAD. |
| [Sep-24 T4/T5](sep-24-adversarial-e2e-bench/README.md) | Benchmark verifier and workloads implemented. | Python contracts passed 20/20 at `db0b03c`; prior 48-row smoke and 220-row full artifacts from `7f1c2f8` passed the revised verifier. A clean `db0b03c` smoke artifact passed 48/48 (SHA-256 `502f6e70a2009f6e1c725a72a9167db8895a25117bb4facfc738fd7532b4f8a4`). Two runs at `fdf34cf` failed the 4.0x revocable fill/drain limit (9.80x and 12.25x); both remain historical artifacts. A later exact-source 220-row run passed; current qualification is represented only by a `qmp-local-qualified-*` tag whose peeled target equals HEAD. |
| Sep-24 T6 | Optional only. | No newly identified invariant justifies adding a daily endurance sweep. |

The [Sep-22 TO-01..09 queue](../../tickets/TEST-OPTIMIZATION-EXECUTION.md)
was implemented and owner-mutant proofs were recorded. The annotated
`qmp-final-qualified-sep23-followup-v5` tag qualified a past source containing
that work. Its old audit tables are historical snapshots, not a current
`receipt pending` queue. The [receipt index](../../tickets/FINAL-CLOSEOUT.md)
explains the exact source boundary.

Hosted CI, deployment, activation, physical power loss, broad B07 metadata
cardinality, and J06 backlog/work limits have no current completion receipt.
The Redb failure-model work is outside the active in-memory release scope.
