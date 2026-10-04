# QMP audit remediation

Baseline: `88093064ccaacaba3f00eb6e22e9d0a33d68ac22`.
Implementation branch: `codex/oct-4-qmp-contracts`.
Scope: portable in-memory accounting, facade ownership, and executable guards.
The original audit checkout is preserved. Its 50-path overlay was copied into
the managed oct-4 worktree; lanes own disjoint files. Current source presence
does not imply a passing checkpoint receipt.

## Structural decisions and acceptance

| Owner | Representation / repair | Required distinguishing observation |
| --- | --- | --- |
| Core policy reconfigure | Live ticket attribution retains class authority even at 0 bytes | Removing optional class denied for pending and committed tickets; cap change allowed; release unblocks removal |
| Broker reclaim | Resident borrowing usage qualifies a victim independently of positive hard cap | Reclaimer actually releases the hard-cap-zero victim, and denied requester then succeeds |
| Scheduler | Validate the actual release axis; keep derived cache internal | Wrong-axis release preserves usage/floor/admission; public root/module access denied; broker benchmark exercises real group admission |
| Payload reference | Sealed injective String codec plus checked store namespace and sequence | Custom lossy codec rejected by compiler; foreign reference cannot delete/pin another store's payload; no unbounded used-reference history |
| Prepared publication | Reserve eventual retirement evidence on fresh prepare | All admitted live entries can abort or batch GC at saturation; new identity denied before charge; existing replay/conflict still resolved |
| Metrics | Thread-local recorder observes real output | Exact metric names, labels, increments, optional byte samples; no-op emitter fails |
| Allocation probe | Callbacks delegate the same saturation path as direct signals | MAX-plus-one remains MAX for both allocation and deallocation |
| Retaining spill restore | Read-unavailable differs from empty/readable; no-op None remains valid | Offline retaining fake rejects, write-full/readable target restores retained FIFO inventory |
| Transcript port | Issued session handle, bounded page and version-conditional whole-session selector | Zero-row caps, stale/foreign/cursor rejection, read pins retain charges, fail-closed retire and independent broken fake |
| Source shape | Rust literals are masked before resolving include/path children | Raw C strings cannot hide compiled source from the 1,000-line ceiling |
| Python contracts | Recursive file inventory matches imported module and class-body origins | Missing discovery, relabeled foreign classes, duplicate IDs, zero-test, skip and import failure reject incomplete execution |
| Architecture / unsafe | Code-token layer guard plus item-scoped benchmark exception | Adapter type/value/grouped/raw-alias paths and crate-root globs rejected; local test globs and strings/comments allowed; unsafe outside delegated allocator denied |
| CI authority | Full gate, dated executor, pinned tools and event-selected checkout | Early agent halt, floating tool versions, checkout provider/ref overrides rejected |

No file is split solely to lower line counts. New tests use semantic owner
modules. Prepared admission moves to its own module because admission and
retirement have different invariants. The existing `core <- facade <- testkit`
graph remains the dependency boundary; no new storage crate is introduced.

## Operating boundaries

- Payload references identify entries only within the issuing process lifetime.
  They are not authorization tokens or cross-process durable identities. Host
  routing must retain its store and process scope. String wire values change;
  consumers must not parse the old `payload-N` spelling.
- Prepared replay evidence persists until close/Drop. Transcript evidence
  persists until its issued handle is retired or its store closes. Removing
  active rows/payload bodies does not erase evidence. Same caller metadata may
  open independent handles; late old-handle operations cannot enter a new one.
  QMP does not infer safe expiration or automatically prune evidence.
- Transcript pages select and pin rows under a stable read, then clone outside
  the mutex. Limits count rows and active semantic costs, not arbitrary Clone
  latency or RSS. CAS selectors describe the whole session, not one page.
  Pinned eviction/retire/close fails before releasing any charge.
- Semantic accounting excludes caller-owned return clones, runtime identity and
  scratch-label copies, transient canonicalization, allocator overhead and RSS.
  Ticket limits bound live authority count, not arbitrary caller string size.
- `spill-redb` stays optional maintenance scope. In-memory release does not
  require storage power-loss qualification or a fourth crate. A production
  backing consumer must own the separate durability contract.
- Static guards reject owned forbidden forms; they are not a general Rust name
  resolver, shell interpreter, or proof of callback safety. Compiler contracts
  and independent runtime observations cover the repaired behavioral boundaries.

## Verification and remaining adoption

Run focused owner tests while editing, then the canonical rail once the
integrated source is stable: `python3 tools/pm/pm.py sync` and
`scripts/check-workspace.sh`. Bind results to the actual source revision.
An extraction check uses a committed clean candidate and the same dependency
contract. Do not substitute a zero-test success or historical receipt.

Expensive scaling, fuzz, Linux storage-fault and remote runner experiments are
not required for these repaired invariants. Hosted execution and Semantica
adoption remain separate: update both consumer workspace roots and lockfiles to
one verified QMP revision, then qualify the real host path. No host cutover or
successful remote run is implied by the local gate.
