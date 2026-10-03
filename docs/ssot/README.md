# QMP contract pointers

QMP owns the portable package graph, public builder API, feature behavior, and
standalone extraction. The executable local contract is
[`scripts/check-workspace.sh`](../../scripts/check-workspace.sh); the package
boundary is described in [ADR 0001](../adr/0001-portable-accounting-authority.md)
and [EXTRACTION.md](../../EXTRACTION.md).

Semantica owns its host adapter and the dependency entries in **both** of its
Cargo workspace roots. Its
[QMP dependency contract](https://github.com/josongmin/semantica-codegraph-v2/blob/main/docs/ssot/quanta-memory-platform-dependency-contract-v1.md)
is the authority for the consumer checkout, Git revision or registry version.
QMP's `MEMORY_PLATFORM_REPO` verification override does not alter Semantica's
Cargo resolution. A QMP repository check does not qualify the host adapter.

For a consumer cutover, first select one clean, verified QMP commit. Change both
Semantica workspace dependency sets, both contract metadata blocks, its contract
document, and the relevant lockfiles together. Resolve Cargo metadata from both
workspace roots to the **same QMP package IDs and source revision**, then run
the host adoption suites against that revision. Keep `quanta-memory-host` outside
this repository.

## Internal release policy

The three workspace crates currently share version `0.2.0` and `publish = false`.
This is a Git-source consumer contract, not a registry publication claim. For
the next Semantica adoption, use the same immutable QMP commit with Cargo
`git` + `rev` entries in both workspace roots. A branch or tag alone does not
freeze resolution. Record the QMP SHA, resolved Cargo package IDs, QMP gate
receipts, Semantica host proof, and Semantica source SHA in the cutover record.

An internal tag may label a qualified commit after hosted and consumer checks;
do not use the tag as the only dependency selector. Registry publication is a
separate cutover requiring registry ownership, a public API compatibility
baseline, registry-ready manifests and source archives, and a version policy.
Remove `publish = false` only as part of that cutover.

`spill-redb` remains an opt-in maintenance feature outside the current in-memory
product scope. The in-memory gate checks that the resolved graph excludes Redb.
Moving the backing to a fourth crate would change the fixed three-crate graph
and the public backing path; consider that migration when a production owner
accepts the durability, filesystem, and recovery qualification contract.
