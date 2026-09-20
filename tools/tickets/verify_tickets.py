#!/usr/bin/env python3
"""Validate the remediation ticket SSOT and its evidence chain."""

from __future__ import annotations

import argparse
import fnmatch
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


TICKET_ID = re.compile(r"^QMP-\d{3}$")
SHA = re.compile(r"^[0-9a-f]{40}$")
RECEIPT_REQUIRED = {"IMPLEMENTED", "QUALIFIED", "DONE"}
TERMINAL = {"QUALIFIED", "DONE"}


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


def validate_receipt(
    text: str,
    status: str,
    current_sha: str | None = None,
    changed_paths: tuple[str, ...] = (),
) -> str:
    head = _field(text, "HEAD")
    base = _field(text, "base SHA")
    source_changed = _field(text, "source changed during run")
    source_paths = _csv_paths(_field(text, "source paths"))
    if not SHA.fullmatch(head) or not SHA.fullmatch(base):
        raise ValidationError("receipt HEAD/base SHA must be full 40-character git SHAs")
    if source_changed != "no":
        raise ValidationError("receipt reports source drift during run")
    for section in ("## RED evidence", "## Verification", "## Acceptance checklist"):
        if section not in text:
            raise ValidationError(f"receipt missing section: {section}")
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

    for ticket in tickets.values():
        receipts = sorted((ticket_root / ticket.path).parent.glob("receipts/*.md"))
        if ticket.status in RECEIPT_REQUIRED and not receipts:
            raise ValidationError(
                f"{ticket.ticket_id} status {ticket.status} requires a receipt"
            )
        for receipt in receipts:
            text = receipt.read_text()
            receipt_head = _field(text, "HEAD")
            changed_paths: tuple[str, ...] = ()
            if current_sha and SHA.fullmatch(receipt_head):
                try:
                    _git(root, "cat-file", "-e", f"{receipt_head}^{{commit}}")
                    changed = _git(root, "diff", "--name-only", f"{receipt_head}..{current_sha}")
                    committed_paths = tuple(changed.splitlines()) if changed else ()
                    changed_paths = tuple(dict.fromkeys((*committed_paths, *dirty_paths)))
                except subprocess.CalledProcessError as exc:
                    raise ValidationError(
                        f"receipt references unknown commit: {receipt_head}"
                    ) from exc
            validate_receipt(text, ticket.status, current_sha, changed_paths)

    claims_dir = ticket_root / "claims"
    claims: list[Claim] = []
    if claims_dir.is_dir():
        claims = [parse_claim(path.read_text()) for path in sorted(claims_dir.glob("*.md"))]
    for index, left in enumerate(claims):
        if left.ticket_id not in tickets:
            raise ValidationError(f"claim references unknown ticket: {left.ticket_id}")
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
    args = parser.parse_args()
    try:
        validate_tree(args.root.resolve())
    except (OSError, ValidationError) as exc:
        print(f"verify-tickets: FAIL: {exc}", file=sys.stderr)
        return 1
    print("verify-tickets: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
