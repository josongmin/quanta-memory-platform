#!/usr/bin/env python3
"""Snapshot or verify actual source tests in the nine sep-26 audit scopes."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent
CORE = "crates/quanta-memory-platform-core/"
FACADE = "crates/quanta-memory-platform-facade/"
KIT = "crates/quanta-memory-platform-testkit/"
SCOPES = {
    "core-domain": [CORE + p for p in [
        "src/domain/ledger.rs", "src/domain/policy.rs", "src/domain/config.rs",
        "src/domain/admission.rs", "tests/policy_contracts.rs",
        "src/domain/context.rs", "src/domain/errors.rs", "src/domain/management_limits.rs",
        "src/domain/mod.rs", "src/domain/recharge.rs", "src/domain/stats.rs",
    ]],
    "core-api": [CORE + p for p in [
        "src/api/control.rs", "src/api/builder.rs", "tests/public_api_surface.rs",
        "src/api/broker.rs", "src/api/mod.rs",
    ]],
    "app-ports": [CORE + p for p in [
        "src/application/controller.rs", "src/application/scheduler.rs",
        "src/adapters/pin.rs", "src/adapters/metrics.rs", "src/adapters/hdrhistogram.rs",
        "src/ports/scratch.rs", "src/ports/telemetry.rs", "src/adapters/scratch.rs",
        "src/adapters/mod.rs", "src/ports/pin.rs", "src/ports/reclaim.rs",
        "src/ports/store.rs", "src/ports/mod.rs", "src/application/mod.rs",
    ]],
    "revocable-store": [FACADE + "src/revocable_store/" + p for p in [
        "service.rs", "durable_codec.rs", "cost.rs", "mod.rs",
    ]],
    "facade-stores": [FACADE + "src/" + module + "/" + file
        for module in ["transcript_store", "retained_map", "payload_store"]
        for file in ["service.rs", "mod.rs"]],
    "spill-redb": [FACADE + "src/spill/" + p for p in [
        "service.rs", "backings/redb.rs", "mod.rs", "backings/mod.rs",
    ]],
    "crash-rails": [FACADE + p for p in [
        "tests/spill_redb_kill_reopen.rs", "tests/spill_redb_finalize_kill.rs",
        "tests/cross_store_hellgate.rs", "tests/hellgate_e2e.rs",
        "examples/spill_redb_crash_child.rs", "examples/spill_redb_finalize_kill_child.rs",
    ]],
    "pipeline-shared": [FACADE + p for p in [
        "src/publish_pipeline/prepared_publication.rs", "src/publish_pipeline/service.rs",
        "src/publish_pipeline/mod.rs", "src/presets/mod.rs", "src/shared/builder.rs",
        "src/shared/lifecycle.rs", "src/shared/measure.rs", "src/shared/accounting.rs",
        "src/shared/mod.rs", "tests/public_api_surface.rs", "src/test_support.rs",
    ]],
    "testkit": [KIT + p for p in [
        "src/conformance/suites.rs", "src/conformance/invariants.rs", "src/conformance/mod.rs",
        "src/conformance/target.rs", "src/faults/scenarios.rs", "src/faults/mod.rs",
        "src/fake/broker.rs", "src/fixtures/mod.rs", "src/alloc_tracking/mod.rs",
        "src/loom_models.rs", "src/lib.rs", "tests/facade_conformance_bite.rs",
        "tests/public_api_surface.rs", "src/fake/mod.rs", "src/fake/views.rs", "src/doctor/mod.rs",
    ]],
}
ATTR_PATTERN = r"^[ \t]*#\[(tokio::)?test(\([^\]\n]*\))?\]"
ATTR_RE = re.compile(ATTR_PATTERN, re.MULTILINE)
FN_RE = re.compile(r"\bfn\s+([A-Za-z_]\w*)\b")
RAW_RE = re.compile(r'(?:br|r)(#*)"')
CHAR_RE = re.compile(r"'(?:\\(?:u\{[0-9A-Fa-f_]+\}|x[0-9A-Fa-f]{2}|[^\n])|[^\\'\n])'")
AUDIT_BASE = "61e9c7c47fa7294f9a926fd867e4af0b116c429b"
HISTORICAL_ACTUAL = dict(zip(SCOPES, [97, 144, 55, 72, 151, 75, 29, 84, 87]))


def command(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).rstrip("\n")


def mask_non_code(text: str) -> str:
    """Mask comments and string/char literals without changing source offsets."""
    result = list(text)
    i = 0
    while i < len(text):
        end = None
        if text.startswith("//", i):
            end = text.find("\n", i)
            if end < 0:
                end = len(text)
        elif text.startswith("/*", i):
            j, depth = i + 2, 1
            while j < len(text) and depth:
                if text.startswith("/*", j):
                    depth += 1
                    j += 2
                elif text.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            if depth:
                raise AssertionError("unterminated block comment")
            end = j
        else:
            raw = RAW_RE.match(text, i)
            if raw and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
                terminator = '"' + raw.group(1)
                j = text.find(terminator, raw.end())
                if j < 0:
                    raise AssertionError("unterminated raw string")
                end = j + len(terminator)
            elif text[i] == '"':
                j = i + 1
                while j < len(text):
                    if text[j] == "\\":
                        j += 2
                    elif text[j] == '"':
                        end = j + 1
                        break
                    else:
                        j += 1
                if end is None:
                    raise AssertionError("unterminated string")
            elif text[i] == "'":
                char = CHAR_RE.match(text, i)
                if char:
                    end = char.end()
        if end is None:
            i += 1
        else:
            for j in range(i, end):
                if result[j] != "\n":
                    result[j] = " "
            i = end
    return "".join(result)


def extract_tests(text: str) -> list[dict]:
    code = mask_non_code(text)
    attrs = list(ATTR_RE.finditer(code))
    tests = []
    for n, attr in enumerate(attrs):
        fn = FN_RE.search(code, attr.end())
        if fn is None or (n + 1 < len(attrs) and fn.start() >= attrs[n + 1].start()):
            raise AssertionError("test attribute has no following function")
        tests.append({
            "name": fn.group(1),
            "attribute": "#[tokio::test]" if attr.group(1) else "#[test]",
            "attribute_line": code.count("\n", 0, attr.start()) + 1,
            "function_line": code.count("\n", 0, fn.start()) + 1,
        })
    return tests


def snapshot() -> dict:
    paths = [p for scope in SCOPES.values() for p in scope]
    assert len(paths) == len(set(paths)), "overlapping scope ownership"
    head = command("git", "rev-parse", "--verify", AUDIT_BASE + "^{commit}")
    dirty = command("git", "status", "--porcelain=v1").splitlines()
    scopes = []
    source_fingerprints = []
    head_test_counts = {}
    head_raw_counts = {}
    for scope, paths in SCOPES.items():
        files = []
        head_count = 0
        head_raw_count = 0
        for path in paths:
            payload = (ROOT / path).read_bytes()
            text = payload.decode("utf-8")
            tests = extract_tests(text)
            baseline = subprocess.run(["git", "show", f"{head}:{path}"], cwd=ROOT,
                                      capture_output=True, text=True, check=False)
            if baseline.returncode == 0:
                head_count += len(extract_tests(baseline.stdout))
                head_raw_count += len(re.findall(r"#\[(?:tokio::)?test\]", baseline.stdout))
            else:
                # This helper was added by remediation and is absent only from
                # the immutable audited base, even after it becomes tracked.
                assert path == FACADE + "src/test_support.rs", (
                    "unexpected file missing from audited base", head, path, baseline.stderr
                )
            rg = subprocess.run(["rg", "-n", ATTR_PATTERN, path], cwd=ROOT,
                                capture_output=True, text=True, check=False)
            assert rg.returncode in (0, 1), rg.stderr
            matches = [line for line in rg.stdout.splitlines() if line]
            assert len(tests) == len(matches), f"{path}: parser={len(tests)}, rg={len(matches)}"
            assert [row["attribute_line"] for row in tests] == [int(line.split(":", 1)[0]) for line in matches], path
            digest = hashlib.sha256(payload).hexdigest()
            source_fingerprints.append(f"{path}\0{digest}\n")
            files.append({"path": path, "sha256": digest,
                          "anchored_rg_count": len(matches), "test_count": len(tests), "tests": tests})
        count = sum(file["test_count"] for file in files)
        head_test_counts[scope] = head_count
        head_raw_counts[scope] = head_raw_count
        assert head_raw_count == HISTORICAL_ACTUAL[scope] + (scope == "core-api"), scope
        assert head_count == HISTORICAL_ACTUAL[scope], f"Audited-base count differs from historical source in {scope}"
        scopes.append({"scope": scope, "historical_actual_count": HISTORICAL_ACTUAL[scope],
                       "test_count": count, "anchored_rg_count": sum(file["anchored_rg_count"] for file in files),
                       "files": files})
    scope_paths = {file["path"] for scope in scopes for file in scope["files"]}
    source_dirty = [line for line in dirty if line[3:] in scope_paths]
    return {"schema": "qmp-current-source-test-census-v1", "head": head,
            "captured_at_utc": datetime.now(timezone.utc).isoformat(),
            "source_state": "dirty" if source_dirty else "clean",
            "source_dirty_status": source_dirty, "head_actual_test_counts": head_test_counts,
            "head_raw_attribute_counts": head_raw_counts,
            "historical_reports": historical_receipt(),
            "source_digest_sha256": hashlib.sha256("".join(sorted(source_fingerprints)).encode()).hexdigest(),
            "total_test_count": sum(scope["test_count"] for scope in scopes),
            "total_anchored_rg_count": sum(scope["anchored_rg_count"] for scope in scopes),
            "scopes": scopes}


def historical_receipt() -> list[dict]:
    reports = []
    placeholder = re.compile(
        r"^\s*(?:TODO|TBD|PLACEHOLDER|WIP|INCOMPLETE|\[insert[^\]]*\])\s*(?::.*)?$",
        re.IGNORECASE | re.MULTILINE,
    )
    for scope, actual in HISTORICAL_ACTUAL.items():
        path = OUT / f"findings-{scope}.md"
        text = path.read_text()
        claimed = actual + (scope == "core-api")
        active, rows = False, []
        for line in text.splitlines():
            if line.startswith("## "):
                active = line[3:].lower().startswith("census")
            if active and line.startswith("|"):
                cells = [cell.strip() for cell in line.strip("|").split("|")]
                if len(cells) >= 4 and any(re.search(r"\b(?:STRONG|WEAK|FAILOPEN|VACUOUS)\b", cell) for cell in cells):
                    rows.append(line)
        assert len(rows) == claimed, (scope, len(rows), claimed)
        assert "## Footer" in text and len(text) > 500, path
        assert not placeholder.search(text), path
        reports.append({"path": str(path.relative_to(ROOT)), "bytes": path.stat().st_size,
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                        "claimed_historical_raw_count": claimed, "actual_census_table_rows": len(rows)})
    for name, final_heading, footer in [
        ("findings-critic.md", "## Safety attestation", "*Critic, 2026-09-26."),
        ("99-index.md", "## 5. Process notes", "*Synthesis step, 2026-09-26."),
    ]:
        path = OUT / name
        text = path.read_text()
        assert len(text) > 500 and final_heading in text and footer in text, path
        assert not placeholder.search(text), path
        reports.append({"path": str(path.relative_to(ROOT)), "bytes": path.stat().st_size,
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    assert sum(report.get("actual_census_table_rows", 0) for report in reports) == 795
    return reports


def test_rows(data: dict) -> list[str]:
    rows = []
    for scope in data["scopes"]:
        for file in scope["files"]:
            for test in file["tests"]:
                rows.append(f'| {len(rows) + 1} | {scope["scope"]} | `{file["path"]}:{test["function_line"]}` | '
                            f'`{test["name"]}` | `{test["attribute"]}` | {test["attribute_line"]} |')
    return rows


def markdown(data: dict) -> str:
    out = ["# Current-source test census — sep-26 fail-open audit", "",
           f'- Immutable audited base commit: `{data["head"]}`.',
           f'- Captured UTC: `{data["captured_at_utc"]}`.',
           f'- Source state at capture: **{data["source_state"]} working tree**; this census inventories the source bytes listed below, not a committed-source qualification receipt.',
           f'- Source digest SHA-256: `{data["source_digest_sha256"]}`.',
           f'- Actual attributed test functions: **{data["total_test_count"]}**; independently anchored `rg` matches: **{data["total_anchored_rg_count"]}**.',
           '- Scope ownership comes from the nine historical reports. The new `facade/src/test_support.rs` belongs to `pipeline-shared`; documented zero-test leftovers and crash-child examples remain inventoried.',
           '- Comments and string literals are excluded before pairing an attribute with its actual function. Each row names the function line and also records its attribute line. Conditional feature gates remain lexical entries; these counts do not assert runtime execution.',
           '- The immutable audited-base source was independently parsed and matches all historical actual per-scope counts. The historical raw count was 795. One doc-comment pseudo-test in `core/tests/public_api_surface.rs` and its non-test compile-pin census row are excluded here, yielding **794 historical actual tests**. Historical reports are preserved.',
           '- Doctests, compile-only helper functions and macro-generated property cases without these attributes are outside this census. Loom model tests are inventoried as source tests, without claiming production concurrency coverage.',
           '', '## Scope totals', '',
           '| Scope | Historical actual tests | Current rows | Anchored rg | Delta | Match |',
           '| --- | ---: | ---: | ---: | ---: | --- |']
    for scope in data["scopes"]:
        out.append(f'| {scope["scope"]} | {scope["historical_actual_count"]} | {scope["test_count"]} | '
                   f'{scope["anchored_rg_count"]} | {scope["test_count"] - scope["historical_actual_count"]:+d} | yes |')
    out += [f'| **Total** | **794** | **{data["total_test_count"]}** | **{data["total_anchored_rg_count"]}** | '
            f'**{data["total_test_count"] - 794:+d}** | **yes** |', '', '## Source file inventory', '',
            '| Scope | Source file | Actual rows | Anchored rg |', '| --- | --- | ---: | ---: |']
    for scope in data["scopes"]:
        for file in scope["files"]:
            out.append(f'| {scope["scope"]} | `{file["path"]}` | {file["test_count"]} | {file["anchored_rg_count"]} |')
    out += ['', '## Historical report completeness', '',
            'The nine historical census tables contain **795 rows**, equal to their historical raw grep claims. All nine findings reports, `findings-critic.md`, and `99-index.md` exist, are nonempty, have concluding sections, and contain no standalone unfinished placeholders. Report hashes are recorded in the JSON and checked by the verifier.', '',
            'This is structural completeness, not acceptance of every finding or a claim that all mutations were executed. The critic explicitly records one safety-skipped WEAK sample (row 12) and one static Loom-reference check (row 9); the synthesis describes the historical audit and its fix checklist, rather than current-source qualification.', '',
            '| Historical report | Census table rows | Raw count claim | Bytes |',
            '| --- | ---: | ---: | ---: |']
    for report in data["historical_reports"]:
        out.append(f'| `{Path(report["path"]).name}` | {report.get("actual_census_table_rows", "n/a")} | '
                   f'{report.get("claimed_historical_raw_count", "n/a")} | {report["bytes"]} |')
    out += ['', '## Test functions', '', '<!-- TEST_CENSUS_START -->',
            '| # | Scope | Source function location | Function name | Attribute | Attribute line |',
            '| ---: | --- | --- | --- | --- | ---: |'] + test_rows(data) + ['<!-- TEST_CENSUS_END -->', '',
            '## Verification', '',
            '```bash', 'python3 docs/bugbash/sep-26-fail-open-audit/verify_current_source_census.py', '```', '',
            'The verifier requires each current source SHA-256 and parsed function row to match the JSON snapshot, independently recomputes the anchored `rg` count per file, and compares every Markdown census row with the JSON inventory. A source change fails verification until this new snapshot is explicitly refreshed with `--write`.', '',
            'Machine-readable snapshot: `current-source-census.json`. The audited base is pinned independently of current HEAD so commits and merges do not redefine the historical count. The newly added test_support.rs is explicitly permitted to be absent from that base even when tracked. No Cargo command is run by this verifier.', '',
            '## Dirty source paths at capture', '', '```text'] + (data["source_dirty_status"] or ['(none)']) + ['```', '']
    return "\n".join(out)


def main() -> None:
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument("--write", action="store_true", help="refresh only the new current-source census artifacts")
    options = args.parse_args()
    current = snapshot()
    json_path = OUT / "current-source-census.json"
    md_path = OUT / "current-source-census.md"
    if options.write:
        json_path.write_text(json.dumps(current, indent=2) + "\n")
        md_path.write_text(markdown(current))
    saved = json.loads(json_path.read_text())
    for field in ["head", "head_actual_test_counts", "head_raw_attribute_counts", "historical_reports", "source_digest_sha256", "total_test_count", "total_anchored_rg_count", "scopes"]:
        assert current[field] == saved[field], f"snapshot drift in {field}; refresh only after reviewing source changes"
    md = md_path.read_text()
    block = md.split("<!-- TEST_CENSUS_START -->", 1)[1].split("<!-- TEST_CENSUS_END -->", 1)[0]
    rows = [line for line in block.splitlines() if re.match(r"^\| \d+ \|", line)]
    assert rows == test_rows(saved), "Markdown rows differ from actual JSON function inventory"
    assert len(rows) == saved["total_test_count"] == saved["total_anchored_rg_count"]
    assert sum(scope["historical_actual_count"] for scope in saved["scopes"]) == 794
    print(f'PASS: {len(saved["scopes"])} scopes; {len(rows)} actual test rows = {saved["total_anchored_rg_count"]} anchored rg matches; source digests and every Markdown/JSON row match.')
    print("PASS: 11 historical reports complete; nine historical census tables = 795 raw rows = historical grep claims; report hashes match.")
    for scope in saved["scopes"]:
        print(f'{scope["scope"]}: rows={scope["test_count"]}, rg={scope["anchored_rg_count"]}, files={len(scope["files"])}')


if __name__ == "__main__":
    main()
