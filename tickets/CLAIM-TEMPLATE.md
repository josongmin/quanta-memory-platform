# `<ticket-id>` active claim

```text
ticket:
claim owner:
base SHA:
write paths: comma-separated repository-relative paths
foreign dirty paths: none, or comma-separated repository-relative paths
start timestamp UTC:
```

Claims live under `tickets/claims/<ticket-id>-<owner>.md` only while active.
The validator rejects unknown tickets, incomplete records, overlapping active
claims, and overlap between a claim's write set and its recorded foreign dirty
paths.
