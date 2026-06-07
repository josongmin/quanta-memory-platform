#!/usr/bin/env python3
"""logq prompt-manager (take-home).

Single source of truth for agent/operator docs. Source content lives under
``tools/pm/sources/**`` and is rendered through jinja2 templates into generated
target files (``AGENTS.md``, ``.codex/*``, ``.cursorrules``, etc.).

Commands:

- ``sync``     render every target and write its output file,
- ``lint``     re-render in memory and fail closed if any output drifted or
               is missing,
- ``status``   list targets, output paths, and ok/drift state,
- ``preview``  print the rendered output for one target without writing.

Generated files carry a "DO NOT EDIT" banner injected by the templates.
The pure functions (``load_targets``, ``render_target``, ``check_drift``) are
importable so the tests can drive them with hermetic fixtures.
"""

from __future__ import annotations

import argparse
import sys
from dataclasses import dataclass
from pathlib import Path

import yaml
from jinja2 import (
    Environment,
    FileSystemLoader,
    StrictUndefined,
    select_autoescape,
)

# tools/pm/pm.py -> PM_DIR = tools/pm, REPO_ROOT = repo root (parent of tools/).
PM_DIR = Path(__file__).resolve().parent
REPO_ROOT = PM_DIR.parent.parent


class PmError(Exception):
    """Raised for configuration/usage errors (missing files, unknown target)."""


@dataclass(frozen=True)
class Target:
    name: str
    output: str
    template: str
    sections: tuple[str, ...]


def load_targets(targets_path: Path) -> dict[str, Target]:
    """Load and validate ``targets.yaml`` into a name -> Target mapping."""
    if not targets_path.is_file():
        raise PmError(f"targets file not found: {targets_path}")
    raw = yaml.safe_load(targets_path.read_text(encoding="utf-8")) or {}
    targets_raw = raw.get("targets")
    if not isinstance(targets_raw, dict):
        raise PmError(f"targets file has no 'targets' mapping: {targets_path}")

    targets: dict[str, Target] = {}
    for name, spec in targets_raw.items():
        if not isinstance(spec, dict):
            raise PmError(f"target '{name}' must be a mapping")
        for key in ("output", "template", "sections"):
            if key not in spec:
                raise PmError(f"target '{name}' missing required key '{key}'")
        sections = spec["sections"]
        if not isinstance(sections, list) or not sections:
            raise PmError(f"target '{name}' must have a non-empty 'sections' list")
        targets[name] = Target(
            name=name,
            output=str(spec["output"]),
            template=str(spec["template"]),
            sections=tuple(str(s) for s in sections),
        )
    return targets


def _read_sections(pm_dir: Path, target: Target) -> str:
    """Concatenate the section source files for ``target``."""
    chunks: list[str] = []
    for section in target.sections:
        section_path = pm_dir / section
        if not section_path.is_file():
            raise PmError(
                f"target '{target.name}': missing source file {section_path}"
            )
        chunks.append(section_path.read_text(encoding="utf-8").rstrip("\n"))
    return "\n\n".join(chunks) + "\n"


def render_target(target: Target, pm_dir: Path = PM_DIR) -> str:
    """Render a single target to a string (does not write to disk)."""
    template_path = pm_dir / target.template
    if not template_path.is_file():
        raise PmError(
            f"target '{target.name}': missing template {template_path}"
        )
    body = _read_sections(pm_dir, target)
    env = Environment(
        loader=FileSystemLoader(str(pm_dir / "templates")),
        autoescape=select_autoescape(enabled_extensions=()),
        undefined=StrictUndefined,
        keep_trailing_newline=True,
    )
    template = env.get_template(Path(target.template).name)
    rendered = template.render(target_name=target.name, output=target.output, body=body)
    if not rendered.endswith("\n"):
        rendered += "\n"
    return rendered


def output_path(target: Target, repo_root: Path = REPO_ROOT) -> Path:
    return repo_root / target.output


def sync_target(target: Target, pm_dir: Path, repo_root: Path) -> Path:
    """Render ``target`` and write its output file. Returns the output path."""
    rendered = render_target(target, pm_dir)
    out = output_path(target, repo_root)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(rendered, encoding="utf-8")
    return out


def check_drift(
    target: Target, pm_dir: Path = PM_DIR, repo_root: Path = REPO_ROOT
) -> str:
    """Return a drift state for ``target``: 'ok', 'drift', or 'missing'."""
    rendered = render_target(target, pm_dir)
    out = output_path(target, repo_root)
    if not out.is_file():
        return "missing"
    current = out.read_text(encoding="utf-8")
    return "ok" if current == rendered else "drift"


def _load(targets_path: Path) -> dict[str, Target]:
    return load_targets(targets_path)


def cmd_sync(args: argparse.Namespace) -> int:
    targets = _load(args.targets)
    for name in sorted(targets):
        out = sync_target(targets[name], args.pm_dir, args.repo_root)
        print(f"synced {name} -> {out}")
    return 0


def cmd_lint(args: argparse.Namespace) -> int:
    targets = _load(args.targets)
    drifted: list[str] = []
    for name in sorted(targets):
        state = check_drift(targets[name], args.pm_dir, args.repo_root)
        if state != "ok":
            drifted.append(f"{name} ({state}) -> {targets[name].output}")
    if drifted:
        print("pm lint FAILED: generated docs are out of date.", file=sys.stderr)
        for line in drifted:
            print(f"  - {line}", file=sys.stderr)
        print(
            "run `uv run python tools/pm/pm.py sync` "
            "(or `python3 tools/pm/pm.py sync`)",
            file=sys.stderr,
        )
        return 1
    print(f"pm lint OK: {len(targets)} target(s) up to date")
    return 0


def cmd_status(args: argparse.Namespace) -> int:
    targets = _load(args.targets)
    print(f"{'TARGET':<14} {'STATE':<8} OUTPUT")
    for name in sorted(targets):
        state = check_drift(targets[name], args.pm_dir, args.repo_root)
        print(f"{name:<14} {state:<8} {targets[name].output}")
    return 0


def cmd_preview(args: argparse.Namespace) -> int:
    targets = _load(args.targets)
    if args.target not in targets:
        raise PmError(
            f"unknown target '{args.target}' "
            f"(known: {', '.join(sorted(targets)) or 'none'})"
        )
    sys.stdout.write(render_target(targets[args.target], args.pm_dir))
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="pm",
        description="logq prompt-manager: render agent/operator docs from sources.",
    )
    parser.add_argument(
        "--targets",
        type=Path,
        default=PM_DIR / "targets.yaml",
        help="path to targets.yaml",
    )
    parser.add_argument(
        "--pm-dir",
        type=Path,
        default=PM_DIR,
        help="directory holding sources/ and templates/",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=REPO_ROOT,
        help="repo root that output paths are resolved against",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("sync", help="render and write all target files").set_defaults(
        func=cmd_sync
    )
    sub.add_parser("lint", help="fail closed if any target drifted").set_defaults(
        func=cmd_lint
    )
    sub.add_parser("status", help="list targets and drift state").set_defaults(
        func=cmd_status
    )
    preview = sub.add_parser("preview", help="print a rendered target to stdout")
    preview.add_argument("--target", required=True, help="target name to preview")
    preview.set_defaults(func=cmd_preview)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except PmError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
