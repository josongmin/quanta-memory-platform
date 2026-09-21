from __future__ import annotations

import sys
import subprocess
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_tickets import (  # noqa: E402
    Claim,
    ValidationError,
    assert_claim_has_no_foreign_overlap,
    parse_inventory,
    source_tree_digest,
    validate_receipt,
    validate_tree,
)
from run_cargo_test import CargoTestEvidenceError, parse_cargo_test_counts  # noqa: E402


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


def claim_text(ticket_id: str = "QMP-000", write_path: str = "tools/tickets") -> str:
    return "\n".join(
        [
            f"ticket: {ticket_id}",
            "claim owner: worker",
            "base SHA: " + "a" * 40,
            f"write paths: {write_path}",
            "foreign dirty paths: none",
            "start timestamp UTC: 2026-09-21T03:10:33Z",
        ]
    )


def receipt_text(
    *,
    ticket_id: str = "QMP-000",
    head: str | None = None,
    base: str | None = None,
    timestamp: str = "2026-09-21T03:11:00Z",
    selected: str = "1",
    exit_code: str = "0",
    from_status: str = "IN_PROGRESS",
    to_status: str = "IMPLEMENTED",
    source_digest: str | None = None,
    output_digest: str | None = None,
    schema: bool = True,
) -> str:
    head = head or "a" * 40
    base = base or "b" * 40
    digest = source_digest or "c" * 64
    output_digest = output_digest or "d" * 64
    fields = [
        f"ticket: {ticket_id}",
        "repository: test",
        f"HEAD: {head}",
        f"base SHA: {base}",
        f"timestamp UTC: {timestamp}",
        "source changed during run: no",
        "source paths: tools/tickets/**",
        f"from: {from_status}",
        f"to: {to_status}",
    ]
    if schema:
        fields.insert(0, "receipt schema: qmp-receipt-v1")
        fields.insert(-2, f"source tree digest: {digest}")
    return "\n".join(
        [
            *fields,
            "## RED evidence",
            "causal negative fixture",
            "## Verification",
            "| Level | Command | Exit | Selected | Result | Source SHA | Output SHA-256 |",
            "| --- | --- | ---: | ---: | --- | --- | --- |",
            f"| FOCUSED | `cargo test -p owner named_selector` | {exit_code} | {selected} | PASS | `{head}` | `{output_digest}` |",
            "## Acceptance checklist",
            "- [x] exact contract",
        ]
    )


def write_ticket_tree(root: Path, *, status: str, claims: tuple[str, ...] = ()) -> Path:
    ticket_root = root / "tickets"
    ticket_root.mkdir()
    (ticket_root / "README.md").write_text(readme([row("QMP-000", status=status)]))
    ticket_dir = ticket_root / "QMP-000-x"
    ticket_dir.mkdir()
    (ticket_dir / "TICKET.md").write_text("# QMP-000\n")
    if claims:
        claims_dir = ticket_root / "claims"
        claims_dir.mkdir()
        for index, claim in enumerate(claims):
            (claims_dir / f"QMP-000-worker-{index}.md").write_text(claim)
    return ticket_dir


def init_git_repo(root: Path) -> str:
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    (root / "seed.txt").write_text("seed\n")
    subprocess.run(["git", "-C", str(root), "add", "seed.txt"], check=True)
    subprocess.run(
        [
            "git",
            "-C",
            str(root),
            "-c",
            "user.name=tests",
            "-c",
            "user.email=tests@example.invalid",
            "commit",
            "-qm",
            "seed",
        ],
        check=True,
    )
    return subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
    ).strip()


class TicketValidatorTests(unittest.TestCase):
    def test_cargo_test_evidence_sums_nonzero_selected_across_targets(self) -> None:
        output = "\n".join(
            [
                "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out",
                "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out",
            ]
        )
        counts = parse_cargo_test_counts(output)
        self.assertEqual(counts.selected, 1)
        self.assertEqual(counts.result_lines, 2)

    def test_cargo_test_evidence_rejects_all_zero_targets(self) -> None:
        output = "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out"
        with self.assertRaisesRegex(CargoTestEvidenceError, "selected zero"):
            parse_cargo_test_counts(output)

    def test_cargo_test_evidence_rejects_missing_result_lines(self) -> None:
        with self.assertRaisesRegex(CargoTestEvidenceError, "no test result lines"):
            parse_cargo_test_counts("Finished test profile")

    def test_inventory_rejects_unknown_status(self) -> None:
        with self.assertRaisesRegex(ValidationError, "invalid ticket status"):
            parse_inventory(readme([row("QMP-000", status="GREENISH")]))

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

    def test_in_progress_requires_exactly_one_claim(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_ticket_tree(root, status="IN_PROGRESS")
            with self.assertRaisesRegex(ValidationError, "IN_PROGRESS.*exactly one claim"):
                validate_tree(root)

    def test_claim_is_rejected_unless_ticket_is_in_progress(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_ticket_tree(root, status="READY", claims=(claim_text(),))
            with self.assertRaisesRegex(ValidationError, "claim.*READY"):
                validate_tree(root)

    def test_multiple_claims_for_one_ticket_are_rejected_even_without_path_overlap(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_ticket_tree(
                root,
                status="IN_PROGRESS",
                claims=(claim_text(write_path="one"), claim_text(write_path="two")),
            )
            with self.assertRaisesRegex(ValidationError, "exactly one claim"):
                validate_tree(root)

    def test_claim_base_must_exist_in_repository(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_git_repo(root)
            write_ticket_tree(root, status="IN_PROGRESS", claims=(claim_text(),))
            with self.assertRaisesRegex(ValidationError, "unknown base commit"):
                validate_tree(root)

    def test_receipt_identity_must_match_ticket_directory(self) -> None:
        with self.assertRaisesRegex(ValidationError, "receipt ticket mismatch"):
            validate_receipt(
                receipt_text(ticket_id="QMP-010"),
                "IMPLEMENTED",
                ticket_id="QMP-000",
                filename="2026-09-21T031100Z-aaaaaaa.md",
            )

    def test_receipt_filename_sha_must_match_full_head(self) -> None:
        with self.assertRaisesRegex(ValidationError, "filename SHA"):
            validate_receipt(
                receipt_text(),
                "IMPLEMENTED",
                ticket_id="QMP-000",
                filename="2026-09-21T031100Z-deadbee.md",
            )

    def test_receipt_timestamp_must_match_filename(self) -> None:
        with self.assertRaisesRegex(ValidationError, "filename timestamp"):
            validate_receipt(
                receipt_text(),
                "IMPLEMENTED",
                ticket_id="QMP-000",
                filename="2026-09-21T031101Z-aaaaaaa.md",
            )

    def test_receipt_base_must_exist_in_repository(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            head = init_git_repo(root)
            ticket_dir = write_ticket_tree(root, status="IMPLEMENTED")
            receipts = ticket_dir / "receipts"
            receipts.mkdir()
            text = receipt_text(
                head=head,
                base="b" * 40,
                timestamp="2026-09-20T00:00:00Z",
                schema=False,
            )
            (receipts / f"2026-09-20T000000Z-{head[:7]}.md").write_text(text)
            with self.assertRaisesRegex(ValidationError, "unknown HEAD/base commit"):
                validate_tree(root)

    def test_v1_focused_evidence_rejects_zero_selected(self) -> None:
        with self.assertRaisesRegex(ValidationError, "selected > 0"):
            validate_receipt(receipt_text(selected="0"), "IMPLEMENTED")

    def test_v1_focused_evidence_requires_numeric_selected_count(self) -> None:
        with self.assertRaisesRegex(ValidationError, "Selected"):
            validate_receipt(receipt_text(selected="n/a"), "IMPLEMENTED")

    def test_v1_evidence_rejects_non_numeric_exit(self) -> None:
        with self.assertRaisesRegex(ValidationError, "Exit"):
            validate_receipt(receipt_text(exit_code="PASS"), "IMPLEMENTED")

    def test_v1_evidence_requires_full_output_digest(self) -> None:
        with self.assertRaisesRegex(ValidationError, "Output SHA-256"):
            validate_receipt(receipt_text(output_digest="short"), "IMPLEMENTED")

    def test_v1_nonzero_focused_evidence_is_accepted(self) -> None:
        self.assertEqual(validate_receipt(receipt_text(), "IMPLEMENTED"), "a" * 40)

    def test_post_cutover_receipt_requires_v1_schema(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ticket_dir = write_ticket_tree(root, status="READY")
            receipts = ticket_dir / "receipts"
            receipts.mkdir()
            (receipts / "2026-09-21T031100Z-aaaaaaa.md").write_text(
                receipt_text(schema=False)
            )
            with self.assertRaisesRegex(ValidationError, "requires qmp-receipt-v1"):
                validate_tree(root)

    def test_latest_transition_receipt_must_match_inventory_status(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            ticket_dir = write_ticket_tree(root, status="IMPLEMENTED")
            receipts = ticket_dir / "receipts"
            receipts.mkdir()
            old = receipt_text(
                timestamp="2026-09-20T00:00:00Z",
                from_status="READY",
                to_status="IMPLEMENTED",
                schema=False,
            )
            new = receipt_text(
                timestamp="2026-09-20T00:00:01Z",
                from_status="IMPLEMENTED",
                to_status="QUALIFIED",
                schema=False,
            )
            (receipts / "2026-09-20T000000Z-aaaaaaa.md").write_text(old)
            (receipts / "2026-09-20T000001Z-aaaaaaa.md").write_text(new)
            with self.assertRaisesRegex(ValidationError, "latest transition.*QUALIFIED"):
                validate_tree(root)

    def test_v1_source_tree_digest_is_recomputed_from_git(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            init_git_repo(root)
            (root / "tools/tickets").mkdir(parents=True)
            (root / "tools/tickets/owner.py").write_text("OWNER = True\n")
            subprocess.run(["git", "-C", str(root), "add", "."], check=True)
            subprocess.run(
                [
                    "git",
                    "-C",
                    str(root),
                    "-c",
                    "user.name=tests",
                    "-c",
                    "user.email=tests@example.invalid",
                    "commit",
                    "-qm",
                    "base",
                ],
                check=True,
            )
            head = subprocess.check_output(
                ["git", "-C", str(root), "rev-parse", "HEAD"], text=True
            ).strip()
            digest = source_tree_digest(root, head, ("tools/tickets/**",))
            valid = receipt_text(head=head, base=head, source_digest=digest)
            self.assertEqual(validate_receipt(valid, "IMPLEMENTED", repo=root), head)
            with self.assertRaisesRegex(ValidationError, "source tree digest mismatch"):
                validate_receipt(receipt_text(head=head, base=head), "IMPLEMENTED", repo=root)

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
