#!/usr/bin/env python3
"""Validate the remediation ticket SSOT and its evidence chain."""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import re
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path


TICKET_ID = re.compile(r"^QMP-\d{3}$")
SHA = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")
RECEIPT_FILENAME = re.compile(
    r"^(?P<timestamp>\d{4}-\d{2}-\d{2}T\d{6}Z)-(?P<sha>[0-9a-f]{7,40})\.md$"
)
VALID_STATUSES = {"READY", "BLOCKED", "IN_PROGRESS", "IMPLEMENTED", "QUALIFIED", "DONE", "REJECTED"}
RECEIPT_REQUIRED = {"IMPLEMENTED", "QUALIFIED", "DONE"}
TERMINAL = {"QUALIFIED", "DONE"}
VALID_EVIDENCE_LEVELS = {"STATIC", "RED", "FOCUSED", "CRATE", "WORKSPACE", "EXTRACTION", "HOST", "PERF"}
RECEIPT_SCHEMA = "qmp-receipt-v1"
RECEIPT_SCHEMA_CUTOVER = datetime(2026, 9, 21, 3, 10, 33, tzinfo=timezone.utc)


class ValidationError(RuntimeError):
    pass


@dataclass(frozen=True)
class Ticket:
    ticket_id: str
    path: str
    status: str
    dependencies: tuple[str, ...]


@dataclass(frozen=True)
class Claim:
    ticket_id: str
    owner: str
    base_sha: str
    write_paths: tuple[str, ...]
    foreign_dirty_paths: tuple[str, ...]
    start_timestamp: str


def _section(text: str, start: str, end: str | None) -> str:
    try:
        body = text.split(start, 1)[1]
    except IndexError as exc:
        raise ValidationError(f"missing section: {start}") from exc
    return body.split(end, 1)[0] if end else body


def _table_rows(section: str) -> list[list[str]]:
    rows: list[list[str]] = []
    for line in section.splitlines():
        if not line.startswith("|") or re.match(r"^\|[ -]+\|", line):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if cells and cells[0] not in {"ID", "Finding"}:
            rows.append(cells)
    return rows


def _expand_dependencies(raw: str, known_ids: set[str]) -> tuple[str, ...]:
    if raw.strip() in {"", "—", "none"}:
        return ()
    result: list[str] = []
    ordered = sorted(known_ids)
    for token in (part.strip() for part in raw.split(",")):
        match = re.fullmatch(r"(QMP-\d{3})\.\.(QMP-\d{3})", token)
        if match:
            start, end = match.groups()
            if start not in known_ids or end not in known_ids:
                result.extend([start, end])
                continue
            lo, hi = ordered.index(start), ordered.index(end)
            if lo > hi:
                raise ValidationError(f"reversed dependency range: {token}")
            result.extend(ordered[lo : hi + 1])
        else:
            result.append(token)
    return tuple(dict.fromkeys(result))


def parse_inventory(readme: str) -> dict[str, Ticket]:
    rows = _table_rows(_section(readme, "## 3. Ticket inventory", "## 4."))
    raw_rows: list[tuple[str, str, str, str]] = []
    seen: set[str] = set()
    for cells in rows:
        if len(cells) < 6:
            raise ValidationError(f"malformed inventory row: {cells}")
        match = re.fullmatch(r"\[(QMP-\d{3})\]\(([^)]+)\)", cells[0])
        if not match:
            raise ValidationError(f"malformed inventory identity: {cells[0]}")
        ticket_id, path = match.groups()
        if ticket_id in seen:
            raise ValidationError(f"duplicate ticket id: {ticket_id}")
        if cells[2] not in VALID_STATUSES:
            raise ValidationError(f"invalid ticket status for {ticket_id}: {cells[2]}")
        seen.add(ticket_id)
        raw_rows.append((ticket_id, path, cells[2], cells[4]))

    tickets: dict[str, Ticket] = {}
    for ticket_id, path, status, raw_dependencies in raw_rows:
        dependencies = _expand_dependencies(raw_dependencies, seen)
        tickets[ticket_id] = Ticket(ticket_id, path, status, dependencies)
    return tickets


def _assert_acyclic(tickets: dict[str, Ticket]) -> None:
    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(ticket_id: str, trail: tuple[str, ...]) -> None:
        if ticket_id in visiting:
            raise ValidationError("dependency cycle: " + " -> ".join((*trail, ticket_id)))
        if ticket_id in visited:
            return
        visiting.add(ticket_id)
        for dependency in tickets[ticket_id].dependencies:
            visit(dependency, (*trail, ticket_id))
        visiting.remove(ticket_id)
        visited.add(ticket_id)

    for ticket_id in tickets:
        visit(ticket_id, ())


def _field(text: str, name: str) -> str:
    match = re.search(rf"(?m)^{re.escape(name)}:\s*(.+?)\s*$", text)
    if not match:
        raise ValidationError(f"missing field: {name}")
    return match.group(1).strip()


def _optional_field(text: str, name: str) -> str | None:
    match = re.search(rf"(?m)^{re.escape(name)}:\s*(.+?)\s*$", text)
    return match.group(1).strip() if match else None


def _csv_paths(raw: str) -> tuple[str, ...]:
    if raw.lower() in {"", "none", "—"}:
        return ()
    return tuple(path.strip() for path in raw.split(",") if path.strip())


def parse_claim(text: str) -> Claim:
    claim = Claim(
        ticket_id=_field(text, "ticket"),
        owner=_field(text, "claim owner"),
        base_sha=_field(text, "base SHA"),
        write_paths=_csv_paths(_field(text, "write paths")),
        foreign_dirty_paths=_csv_paths(_field(text, "foreign dirty paths")),
        start_timestamp=_field(text, "start timestamp UTC"),
    )
    if not TICKET_ID.fullmatch(claim.ticket_id):
        raise ValidationError(f"invalid claim ticket: {claim.ticket_id}")
    if not SHA.fullmatch(claim.base_sha):
        raise ValidationError(f"invalid claim base SHA: {claim.base_sha}")
    if not claim.owner or not claim.write_paths or not claim.start_timestamp:
        raise ValidationError("claim is missing owner, write paths, or timestamp")
    assert_claim_has_no_foreign_overlap(claim)
    return claim


def _paths_overlap(left: str, right: str) -> bool:
    left = left.rstrip("/")
    right = right.rstrip("/")
    return left == right or left.startswith(right + "/") or right.startswith(left + "/")


def assert_claim_has_no_foreign_overlap(claim: Claim) -> None:
    for write_path in claim.write_paths:
        for dirty_path in claim.foreign_dirty_paths:
            if _paths_overlap(write_path, dirty_path):
                raise ValidationError(
                    f"claim {claim.ticket_id} overlaps foreign dirty path: "
                    f"{write_path} vs {dirty_path}"
                )


def _git(repo: Path, *args: str) -> str:
    completed = subprocess.run(
        ["git", "-C", str(repo), *args],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return completed.stdout.strip()


def _path_matches(path: str, patterns: tuple[str, ...]) -> bool:
    return any(
        fnmatch.fnmatchcase(path, pattern)
        if any(char in pattern for char in "*?[")
        else _paths_overlap(path, pattern)
        for pattern in patterns
    )


def source_tree_digest(repo: Path, head: str, source_paths: tuple[str, ...]) -> str:
    """Digest committed mode/blob/path identities for one receipt source scope."""
    if not source_paths:
        raise ValidationError("v1 receipt source paths must not be empty")
    completed = subprocess.run(
        ["git", "-C", str(repo), "ls-tree", "-rz", "--full-tree", head],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    entries: list[tuple[bytes, bytes, bytes]] = []
    for raw in completed.stdout.split(b"\0"):
        if not raw:
            continue
        metadata, raw_path = raw.split(b"\t", 1)
        mode, object_type, object_id = metadata.split(b" ", 2)
        if object_type != b"blob":
            continue
        path = raw_path.decode("utf-8")
        if _path_matches(path, source_paths):
            entries.append((mode, object_id, raw_path))
    if not entries:
        raise ValidationError("v1 receipt source paths match no committed blobs")
    digest = hashlib.sha256()
    for mode, object_id, raw_path in sorted(entries, key=lambda entry: entry[2]):
        for item in (mode, object_id, raw_path):
            digest.update(len(item).to_bytes(8, "big"))
            digest.update(item)
    return digest.hexdigest()


def _receipt_filename_parts(filename: str) -> tuple[datetime, str, str]:
    match = RECEIPT_FILENAME.fullmatch(filename)
    if not match:
        raise ValidationError(f"invalid receipt filename: {filename}")
    compact = match.group("timestamp")
    timestamp = datetime.strptime(compact, "%Y-%m-%dT%H%M%SZ").replace(tzinfo=timezone.utc)
    canonical = timestamp.strftime("%Y-%m-%dT%H:%M:%SZ")
    return timestamp, canonical, match.group("sha")


def _verification_rows(text: str) -> list[list[str]]:
    section = _section(text, "## Verification", "## Acceptance checklist")
    table_lines = [line for line in section.splitlines() if line.startswith("|")]
    if len(table_lines) < 3:
        raise ValidationError("v1 receipt verification table is missing")
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in table_lines
        if not all(set(cell.strip()) <= {"-", ":"} for cell in line.strip().strip("|").split("|"))
    ]
    expected = [
        "Level",
        "Command",
        "Exit",
        "Selected",
        "Result",
        "Source SHA",
        "Output SHA-256",
    ]
    if not rows or rows[0] != expected:
        raise ValidationError("v1 Verification header must be: " + " | ".join(expected))
    return rows[1:]


def _validate_v1_receipt(text: str, head: str, repo: Path | None) -> None:
    source_paths = _csv_paths(_field(text, "source paths"))
    recorded_digest = _field(text, "source tree digest")
    if not SHA256.fullmatch(recorded_digest):
        raise ValidationError("v1 receipt source tree digest must be SHA-256")
    from_status = _field(text, "from")
    to_status = _field(text, "to")
    if from_status not in VALID_STATUSES or to_status not in VALID_STATUSES:
        raise ValidationError("v1 receipt transition uses an invalid status")
    rows = _verification_rows(text)
    if not rows:
        raise ValidationError("v1 receipt has no executed verification rows")
    for cells in rows:
        if len(cells) != 7:
            raise ValidationError(f"malformed v1 verification row: {cells}")
        level, command, exit_code, selected, result, source_sha, output_sha = cells
        if level not in VALID_EVIDENCE_LEVELS:
            raise ValidationError(f"invalid evidence level: {level}")
        if not command.strip("`") or not result:
            raise ValidationError("v1 verification command/result must not be empty")
        if not re.fullmatch(r"\d+", exit_code):
            raise ValidationError(f"v1 verification Exit must be a non-negative integer: {exit_code}")
        if level == "FOCUSED":
            if not re.fullmatch(r"\d+", selected):
                raise ValidationError("v1 FOCUSED verification Selected must be numeric")
            if int(selected) <= 0:
                raise ValidationError("v1 FOCUSED verification requires selected > 0")
        elif selected != "n/a" and not re.fullmatch(r"\d+", selected):
            raise ValidationError(f"v1 verification Selected must be numeric or n/a: {selected}")
        normalized_source = source_sha.strip("`")
        if not SHA.fullmatch(normalized_source):
            raise ValidationError(f"v1 verification Source SHA must be full: {source_sha}")
        normalized_output = output_sha.strip("`")
        if not SHA256.fullmatch(normalized_output):
            raise ValidationError(
                f"v1 verification Output SHA-256 must be full: {output_sha}"
            )
        if repo is not None:
            try:
                _git(repo, "cat-file", "-e", f"{normalized_source}^{{commit}}")
            except subprocess.CalledProcessError as exc:
                raise ValidationError(
                    f"v1 verification references unknown source commit: {normalized_source}"
                ) from exc
    if repo is not None:
        actual_digest = source_tree_digest(repo, head, source_paths)
        if recorded_digest != actual_digest:
            raise ValidationError(
                f"source tree digest mismatch: recorded {recorded_digest}, actual {actual_digest}"
            )


def validate_receipt(
    text: str,
    status: str,
    current_sha: str | None = None,
    changed_paths: tuple[str, ...] = (),
    *,
    ticket_id: str | None = None,
    filename: str | None = None,
    repo: Path | None = None,
    strict_required: bool = False,
) -> str:
    schema = _optional_field(text, "receipt schema")
    if schema is not None and schema != RECEIPT_SCHEMA:
        raise ValidationError(f"unsupported receipt schema: {schema}")
    if strict_required and schema != RECEIPT_SCHEMA:
        raise ValidationError(f"post-cutover receipt requires {RECEIPT_SCHEMA}")
    recorded_ticket = _optional_field(text, "ticket")
    if ticket_id is not None and recorded_ticket != ticket_id:
        raise ValidationError(
            f"receipt ticket mismatch: directory {ticket_id}, receipt {recorded_ticket!r}"
        )
    head = _field(text, "HEAD")
    base = _field(text, "base SHA")
    source_changed = _field(text, "source changed during run")
    source_paths = _csv_paths(_field(text, "source paths"))
    if not SHA.fullmatch(head) or not SHA.fullmatch(base):
        raise ValidationError("receipt HEAD/base SHA must be full 40-character git SHAs")
    if filename is not None:
        _, filename_timestamp, filename_sha = _receipt_filename_parts(filename)
        if not head.startswith(filename_sha):
            raise ValidationError(
                f"receipt filename SHA {filename_sha} does not match HEAD {head}"
            )
        recorded_timestamp = _field(text, "timestamp UTC")
        if recorded_timestamp != filename_timestamp:
            raise ValidationError(
                f"receipt filename timestamp {filename_timestamp} does not match {recorded_timestamp}"
            )
    if source_changed != "no":
        raise ValidationError("receipt reports source drift during run")
    for section in ("## RED evidence", "## Verification", "## Acceptance checklist"):
        if section not in text:
            raise ValidationError(f"receipt missing section: {section}")
    if schema == RECEIPT_SCHEMA:
        _validate_v1_receipt(text, head, repo)
    if status in TERMINAL and current_sha and head != current_sha:
        scoped_drift = [
            path
            for path in changed_paths
            if any(
                fnmatch.fnmatchcase(path, pattern)
                if any(char in pattern for char in "*?[")
                else _paths_overlap(path, pattern)
                for pattern in source_paths
            )
        ]
        if scoped_drift:
            raise ValidationError(
                "terminal receipt is stale; owned source changed after receipt HEAD: "
                + ", ".join(scoped_drift)
            )
    return head


def validate_tree(root: Path) -> None:
    ticket_root = root / "tickets"
    readme = (ticket_root / "README.md").read_text()
    tickets = parse_inventory(readme)
    if not tickets:
        raise ValidationError("ticket inventory is empty")

    listed_paths: set[str] = set()
    for ticket in tickets.values():
        path = ticket_root / ticket.path
        listed_paths.add(ticket.path)
        if not path.is_file():
            raise ValidationError(f"missing ticket file: {ticket.path}")
        heading = path.read_text().splitlines()[0]
        if ticket.ticket_id not in heading:
            raise ValidationError(f"ticket heading/id mismatch: {ticket.path}")
        for dependency in ticket.dependencies:
            if dependency not in tickets:
                raise ValidationError(
                    f"{ticket.ticket_id} depends on unknown ticket {dependency}"
                )

    actual_paths = {
        str(path.relative_to(ticket_root))
        for path in ticket_root.glob("QMP-*/TICKET.md")
    }
    unlisted = sorted(actual_paths - listed_paths)
    if unlisted:
        raise ValidationError("ticket files missing from inventory: " + ", ".join(unlisted))
    _assert_acyclic(tickets)

    crosswalk = _table_rows(_section(readme, "## 4. 감사 finding crosswalk", "## 5."))
    for cells in crosswalk:
        if len(cells) < 3:
            raise ValidationError(f"malformed finding row: {cells}")
        owners = set(re.findall(r"QMP-\d{3}", cells[1]))
        if not owners:
            raise ValidationError(f"finding has no owning ticket: {cells[0]}")
        unknown = owners - tickets.keys()
        if unknown:
            raise ValidationError(
                f"finding {cells[0]} has unknown owner(s): {', '.join(sorted(unknown))}"
            )

    try:
        current_sha = _git(root, "rev-parse", "HEAD")
        dirty_output = _git(root, "status", "--porcelain")
        dirty_paths = tuple(
            line[3:].split(" -> ")[-1]
            for line in dirty_output.splitlines()
            if len(line) > 3
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        current_sha = None
        dirty_paths = ()

    claims_dir = ticket_root / "claims"
    claims: list[Claim] = []
    if claims_dir.is_dir():
        for path in sorted(claims_dir.glob("*.md")):
            claim = parse_claim(path.read_text())
            if not path.name.startswith(claim.ticket_id + "-"):
                raise ValidationError(
                    f"claim filename does not match ticket {claim.ticket_id}: {path.name}"
                )
            claims.append(claim)
    claims_by_ticket: dict[str, list[Claim]] = {ticket_id: [] for ticket_id in tickets}
    for claim in claims:
        if claim.ticket_id not in tickets:
            raise ValidationError(f"claim references unknown ticket: {claim.ticket_id}")
        claims_by_ticket[claim.ticket_id].append(claim)
        if current_sha:
            try:
                _git(root, "cat-file", "-e", f"{claim.base_sha}^{{commit}}")
            except subprocess.CalledProcessError as exc:
                raise ValidationError(
                    f"claim references unknown base commit: {claim.base_sha}"
                ) from exc
    for ticket in tickets.values():
        count = len(claims_by_ticket[ticket.ticket_id])
        if ticket.status == "IN_PROGRESS" and count != 1:
            raise ValidationError(
                f"{ticket.ticket_id} is IN_PROGRESS and requires exactly one claim; found {count}"
            )
        if ticket.status != "IN_PROGRESS" and count:
            raise ValidationError(
                f"claim exists for {ticket.ticket_id} while status is {ticket.status}"
            )

    for ticket in tickets.values():
        receipts = sorted((ticket_root / ticket.path).parent.glob("receipts/*.md"))
        if ticket.status in RECEIPT_REQUIRED and not receipts:
            raise ValidationError(
                f"{ticket.ticket_id} status {ticket.status} requires a receipt"
            )
        transitions: list[tuple[str, str]] = []
        for receipt in receipts:
            text = receipt.read_text()
            receipt_head = _field(text, "HEAD")
            changed_paths: tuple[str, ...] = ()
            if current_sha and SHA.fullmatch(receipt_head):
                try:
                    _git(root, "cat-file", "-e", f"{receipt_head}^{{commit}}")
                    receipt_base = _field(text, "base SHA")
                    _git(root, "cat-file", "-e", f"{receipt_base}^{{commit}}")
                    changed = _git(root, "diff", "--name-only", f"{receipt_head}..{current_sha}")
                    committed_paths = tuple(changed.splitlines()) if changed else ()
                    changed_paths = tuple(dict.fromkeys((*committed_paths, *dirty_paths)))
                except subprocess.CalledProcessError as exc:
                    raise ValidationError(
                        f"receipt references unknown HEAD/base commit: {receipt.name}"
                    ) from exc
            receipt_timestamp, _, _ = _receipt_filename_parts(receipt.name)
            validate_receipt(
                text,
                ticket.status,
                current_sha,
                changed_paths,
                ticket_id=ticket.ticket_id,
                filename=receipt.name,
                repo=root if current_sha else None,
                strict_required=receipt_timestamp >= RECEIPT_SCHEMA_CUTOVER,
            )
            transition_to = _optional_field(text, "to")
            if transition_to is not None:
                if transition_to not in VALID_STATUSES:
                    raise ValidationError(
                        f"receipt {receipt.name} has invalid transition status: {transition_to}"
                    )
                transitions.append((receipt.name, transition_to))
        if ticket.status in RECEIPT_REQUIRED:
            if not transitions:
                raise ValidationError(
                    f"{ticket.ticket_id} status {ticket.status} has no transition receipt"
                )
            latest_name, latest_status = transitions[-1]
            if latest_status != ticket.status:
                raise ValidationError(
                    f"{ticket.ticket_id} latest transition {latest_name} is {latest_status}, "
                    f"inventory is {ticket.status}"
                )

    for index, left in enumerate(claims):
        for right in claims[index + 1 :]:
            for left_path in left.write_paths:
                for right_path in right.write_paths:
                    if _paths_overlap(left_path, right_path):
                        raise ValidationError(
                            f"active claims overlap: {left.ticket_id}:{left_path} and "
                            f"{right.ticket_id}:{right_path}"
                        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument("--source-digest-head")
    parser.add_argument("--source-path", action="append", default=[])
    args = parser.parse_args()
    root = args.root.resolve()
    try:
        if args.source_digest_head:
            if not args.source_path:
                raise ValidationError("--source-digest-head requires at least one --source-path")
            print(source_tree_digest(root, args.source_digest_head, tuple(args.source_path)))
            return 0
        if args.source_path:
            raise ValidationError("--source-path requires --source-digest-head")
        validate_tree(root)
    except (OSError, ValidationError) as exc:
        print(f"verify-tickets: FAIL: {exc}", file=sys.stderr)
        return 1
    print("verify-tickets: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
