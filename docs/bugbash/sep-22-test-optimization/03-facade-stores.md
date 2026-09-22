# Track 3 — Verified facade-store work

Snapshot: `main@101781c2ea6122602b4acd9aa69ebfc13fda958f`.

## TO-02A — Remove source-text architecture oracles

The following tests grep implementation text rather than compile or execute the
contract:

- `crates/quanta-memory-platform-facade/src/payload_store/service.rs:1220-1232`
- `crates/quanta-memory-platform-facade/src/retained_map/service.rs:755-773`
- `crates/quanta-memory-platform-facade/src/revocable_store/service.rs:1175-1187`

They can miss equivalent forbidden APIs spelled differently and can fail on
comments or lawful refactors. The public types already carry `compile_fail`
examples for raw construction and late `charged_against` attachment.

Change: delete the text greps after ensuring the corresponding rustdoc negative
examples are selected by the documented test rail. If a performance invariant
such as O(1) point lookup still needs enforcement, cover it with the existing
probe counters or a benchmark, not a container-name grep.

## TO-09 — Strengthen transcript byte-partition stats

- Evidence: `crates/quanta-memory-platform-facade/src/transcript_store/service.rs:1471-1493`.
- `retention_stats_separate_active_evidence_and_session_costs` checks the three
  byte partitions only with `> 0`; a regression moving bytes between evidence
  and session-key buckets can pass while the total remains correct.
- Change: use the test module's independent `cost` component constants to assert
  exact active, evidence, and session-key bytes before truncation, then retain
  the existing transition assertions.
- Do not call the production cost calculator from the oracle.

No size/thread reductions remain here. Warm exact measurements were 0.01–0.08 s
for the previously flagged payload probe, retained-map race/probe, transcript
race, and large-identity spill tests. The subprocess tests are required crash
isolation and measured 0.16–0.25 s.
