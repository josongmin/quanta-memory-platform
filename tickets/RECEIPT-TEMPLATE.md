# `<ticket-id>` qualification receipt — qmp-receipt-v1

Cutover rule: every receipt timestamped at or after
`2026-09-21T03:10:33Z` must use this exact schema. Historical receipts remain
evidence and are validated for identity/transition consistency.

## Identity

```text
receipt schema: qmp-receipt-v1
ticket:
repository: quanta-memory-platform
branch:
HEAD: full 40-character implementation/proof commit
base SHA: full 40-character base commit
timestamp UTC: YYYY-MM-DDTHH:MM:SSZ; must equal filename timestamp
toolchain:
host:
source changed during run: no
source paths: comma-separated repository-relative paths or globs
source tree digest: SHA-256 emitted by the command below
from: READY/BLOCKED/IN_PROGRESS/IMPLEMENTED/QUALIFIED/DONE/REJECTED
to: READY/BLOCKED/IN_PROGRESS/IMPLEMENTED/QUALIFIED/DONE/REJECTED
```

The filename is `<UTC-without-colons>-<HEAD-prefix>.md`; its timestamp and SHA
must match the fields above. Compute the committed source identity with:

```bash
python3 tools/tickets/verify_tickets.py \
  --source-digest-head <full-HEAD> \
  --source-path '<owned-path-or-glob>'
```

## Ownership

```text
claimed by:
write set:
foreign dirty paths:
```

## Change summary

- Canonical owner modified:
- Producer -> contract -> consumer wiring:
- Public API impact:
- Migration performed:

## RED evidence

```text
selector:
pre-fix exit:
observed failure:
expected causal failure:
```

## Verification

Every row represents an executed command. `Exit` and `Selected` are numeric.
Every `FOCUSED` row requires `Selected > 0`; use `n/a` only for non-focused
commands. Use `scripts/run-ticket-test` for Cargo selectors so an exit-0
zero-test selection becomes a terminal failure. Record the emitted
`output_sha256`; every v1 row requires a full output digest.

| Level | Command | Exit | Selected | Result | Source SHA | Output SHA-256 |
| --- | --- | ---: | ---: | --- | --- | --- |
| FOCUSED | `scripts/run-ticket-test -p <owner> <selector>` |  |  |  | `<full-HEAD>` | `<full-output-digest>` |
| WORKSPACE |  |  | n/a |  | `<full-HEAD>` | `<full-output-digest>` |

## Acceptance checklist

- [ ] Every ticket acceptance condition is evidenced.
- [ ] Required negative test fails before fix and passes after fix.
- [ ] No assertion was weakened to obtain GREEN.
- [ ] No consumer shim or duplicate accounting authority was added.
- [ ] Public API/docs/examples changed where required.
- [ ] Final worktree ownership is recorded.

## Not run / residual risk

- Not run:
- Reason:
- Residual risk:
- Follow-up ticket:
