"""Hermetic tests for the logq prompt-manager.

Each test builds a tiny synthetic pm tree (targets.yaml + sources + templates)
under tmp_path, so nothing depends on the real generated files.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import pm  # noqa: E402

TARGETS_YAML = """\
targets:
  demo:
    output: out/DEMO.md
    template: templates/demo.j2
    sections:
      - sources/a.md
      - sources/b.md
"""

TEMPLATE = "BANNER DO NOT EDIT\n{{ body }}"


def _build_tree(tmp_path: Path) -> tuple[Path, Path, Path]:
    pm_dir = tmp_path / "pm"
    repo = tmp_path / "repo"
    (pm_dir / "sources").mkdir(parents=True)
    (pm_dir / "templates").mkdir(parents=True)
    repo.mkdir()

    targets = pm_dir / "targets.yaml"
    targets.write_text(TARGETS_YAML, encoding="utf-8")
    (pm_dir / "sources" / "a.md").write_text("alpha", encoding="utf-8")
    (pm_dir / "sources" / "b.md").write_text("beta", encoding="utf-8")
    (pm_dir / "templates" / "demo.j2").write_text(TEMPLATE, encoding="utf-8")
    return targets, pm_dir, repo


def test_load_targets_parses_spec(tmp_path: Path):
    targets, _pm_dir, _repo = _build_tree(tmp_path)
    loaded = pm.load_targets(targets)
    assert set(loaded) == {"demo"}
    demo = loaded["demo"]
    assert demo.output == "out/DEMO.md"
    assert demo.template == "templates/demo.j2"
    assert demo.sections == ("sources/a.md", "sources/b.md")


def test_render_target_includes_banner_and_sections(tmp_path: Path):
    targets, pm_dir, _repo = _build_tree(tmp_path)
    demo = pm.load_targets(targets)["demo"]
    rendered = pm.render_target(demo, pm_dir)
    assert "DO NOT EDIT" in rendered
    assert "alpha" in rendered
    assert "beta" in rendered


def test_missing_source_file_raises(tmp_path: Path):
    targets, pm_dir, _repo = _build_tree(tmp_path)
    (pm_dir / "sources" / "a.md").unlink()
    demo = pm.load_targets(targets)["demo"]
    with pytest.raises(pm.PmError, match="missing source file"):
        pm.render_target(demo, pm_dir)


def test_missing_template_raises(tmp_path: Path):
    targets, pm_dir, _repo = _build_tree(tmp_path)
    (pm_dir / "templates" / "demo.j2").unlink()
    demo = pm.load_targets(targets)["demo"]
    with pytest.raises(pm.PmError, match="missing template"):
        pm.render_target(demo, pm_dir)


def test_unknown_target_in_preview_raises(tmp_path: Path):
    targets, pm_dir, repo = _build_tree(tmp_path)
    args = pm.build_parser().parse_args(
        [
            "--targets",
            str(targets),
            "--pm-dir",
            str(pm_dir),
            "--repo-root",
            str(repo),
            "preview",
            "--target",
            "nope",
        ]
    )
    with pytest.raises(pm.PmError, match="unknown target"):
        args.func(args)


def test_drift_states_missing_then_ok_then_drift(tmp_path: Path):
    targets, pm_dir, repo = _build_tree(tmp_path)
    demo = pm.load_targets(targets)["demo"]

    assert pm.check_drift(demo, pm_dir, repo) == "missing"

    pm.sync_target(demo, pm_dir, repo)
    assert pm.check_drift(demo, pm_dir, repo) == "ok"

    out = pm.output_path(demo, repo)
    out.write_text("tampered\n", encoding="utf-8")
    assert pm.check_drift(demo, pm_dir, repo) == "drift"


def test_lint_command_fails_closed_on_drift(tmp_path: Path):
    targets, pm_dir, repo = _build_tree(tmp_path)
    parser = pm.build_parser()
    base = [
        "--targets",
        str(targets),
        "--pm-dir",
        str(pm_dir),
        "--repo-root",
        str(repo),
    ]

    lint_args = parser.parse_args(base + ["lint"])
    assert lint_args.func(lint_args) == 1

    sync_args = parser.parse_args(base + ["sync"])
    assert sync_args.func(sync_args) == 0
    assert lint_args.func(lint_args) == 0


def test_preview_is_dry_run_and_writes_nothing(tmp_path: Path, capsys):
    targets, pm_dir, repo = _build_tree(tmp_path)
    demo = pm.load_targets(targets)["demo"]
    out = pm.output_path(demo, repo)
    assert not out.exists()

    args = pm.build_parser().parse_args(
        [
            "--targets",
            str(targets),
            "--pm-dir",
            str(pm_dir),
            "--repo-root",
            str(repo),
            "preview",
            "--target",
            "demo",
        ]
    )
    assert args.func(args) == 0
    captured = capsys.readouterr()
    assert "alpha" in captured.out
    assert not out.exists()
