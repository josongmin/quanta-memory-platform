# `<ticket-id>` qualification receipt

## Identity

```text
ticket:
repository:
branch:
HEAD:
base SHA:
timestamp UTC:
toolchain:
host:
```

## Ownership

```text
claimed by:
write set:
foreign dirty paths:
source changed during run: yes/no
source paths: comma-separated repository-relative paths or globs owned by this receipt
```

## Change summary

- Canonical owner modified:
- Producer → contract → consumer wiring:
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

| Level | Command | Exit | Counts/result | Source SHA |
| --- | --- | ---: | --- | --- |
| STATIC |  |  |  |  |
| FOCUSED |  |  |  |  |
| CRATE |  |  |  |  |
| WORKSPACE |  |  |  |  |
| EXTRACTION |  |  |  |  |
| HOST |  |  |  |  |
| PERF |  |  |  |  |

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

## Proposed lifecycle transition

```text
from:
to:
rationale:
```
