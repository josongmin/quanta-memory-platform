from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_tickets import (  # noqa: E402
    Claim,
    ValidationError,
    assert_claim_has_no_foreign_overlap,
    parse_inventory,
    validate_receipt,
    validate_tree,
)


def readme(rows: list[str], crosswalk: str = "| P1-01 defect | QMP-000 | proof |") -> str:
    return "\n".join(
        [
            "## 3. Ticket inventory",
            "| ID | 우선순위 | 상태 | 목적 | depends_on | 주요 write set |",
            "| --- | --- | --- | --- | --- | --- |",
            *rows,
            "## 4. 감사 finding crosswalk",
            "| Finding | Ticket | Closure proof |",
            "| --- | --- | --- |",
            crosswalk,
            "## 5. Dependency graph",
        ]
    )


def row(ticket_id: str, dependency: str = "—", status: str = "READY") -> str:
    return (
        f"| [{ticket_id}]({ticket_id}-x/TICKET.md) | P1 | {status} | purpose | "
        f"{dependency} | path |"
    )


class TicketValidatorTests(unittest.TestCase):
    def test_ticket_inventory_rejects_missing_ticket_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "tickets").mkdir()
            (root / "tickets/README.md").write_text(readme([row("QMP-000")]))
            with self.assertRaisesRegex(ValidationError, "missing ticket file"):
                validate_tree(root)

    def test_ticket_inventory_rejects_duplicate_id(self) -> None:
        with self.assertRaisesRegex(ValidationError, "duplicate ticket id"):
            parse_inventory(readme([row("QMP-000"), row("QMP-000")]))

    def test_ticket_dependencies_are_acyclic(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            tickets = root / "tickets"
            tickets.mkdir()
            rows = [row("QMP-000", "QMP-010"), row("QMP-010", "QMP-000")]
            (tickets / "README.md").write_text(readme(rows))
            for ticket_id in ("QMP-000", "QMP-010"):
                path = tickets / f"{ticket_id}-x"
                path.mkdir()
                (path / "TICKET.md").write_text(f"# {ticket_id}\n")
            with self.assertRaisesRegex(ValidationError, "dependency cycle"):
                validate_tree(root)

    def test_every_confirmed_finding_has_an_owner(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ticket_root = root / "tickets"
            ticket_root.mkdir()
            (ticket_root / "README.md").write_text(
                readme([row("QMP-000")], "| P1-01 defect | none | proof |")
            )
            path = ticket_root / "QMP-000-x"
            path.mkdir()
            (path / "TICKET.md").write_text("# QMP-000\n")
            with self.assertRaisesRegex(ValidationError, "no owning ticket"):
                validate_tree(root)

    def test_receipt_rejects_source_sha_drift(self) -> None:
        receipt = "\n".join(
            [
                "HEAD: " + "a" * 40,
                "base SHA: " + "b" * 40,
                "source changed during run: no",
                "source paths: crates/core/**",
                "## RED evidence",
                "## Verification",
                "## Acceptance checklist",
            ]
        )
        with self.assertRaisesRegex(ValidationError, "stale"):
            validate_receipt(receipt, "QUALIFIED", "c" * 40, ("crates/core/src/lib.rs",))

    def test_claim_rejects_overlapping_foreign_dirty_path(self) -> None:
        claim = Claim(
            ticket_id="QMP-010",
            owner="worker",
            base_sha="a" * 40,
            write_paths=("crates/core",),
            foreign_dirty_paths=("crates/core/src/lib.rs",),
            start_timestamp="2026-09-20T00:00:00Z",
        )
        with self.assertRaisesRegex(ValidationError, "overlaps foreign dirty path"):
            assert_claim_has_no_foreign_overlap(claim)

    def test_implemented_without_required_receipt_is_not_terminal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ticket_root = root / "tickets"
            ticket_root.mkdir()
            (ticket_root / "README.md").write_text(
                readme([row("QMP-000", status="IMPLEMENTED")])
            )
            path = ticket_root / "QMP-000-x"
            path.mkdir()
            (path / "TICKET.md").write_text("# QMP-000\n")
            with self.assertRaisesRegex(ValidationError, "requires a receipt"):
                validate_tree(root)


if __name__ == "__main__":
    unittest.main()
