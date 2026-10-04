"""Fail-closed shape checks for the hosted QMP verification entry point.

These do not claim a CircleCI runner executed. The extracted full gate and
examples remain the only normal job command.
"""

from __future__ import annotations

import hashlib
import re
import unittest
from pathlib import Path

try:
    import tomllib
except ImportError:
    import tomli as tomllib

ROOT = Path(__file__).resolve().parents[2]
CONFIG = ROOT / ".circleci/config.yml"
EXAMPLES = ROOT / "scripts/verify-extracted-examples.sh"
FULL_STEP = "Verify actual standalone extraction and full workspace gate"
FULL_COMMAND = "scripts/verify-extracted-examples.sh"
SHA_CHECK = 'test "$(git rev-parse HEAD)" = "$CIRCLE_SHA1"'
IMAGE = "ubuntu-2404:2026.05.1"

# One digest per approved YAML step, including its shell body and cache keys.
# A deliberate CI edit requires reviewing the whole affected step and updating
# its digest. The explicit sequence also rejects inserted steps and checkout
# mappings before the mandatory verification command.
APPROVED_STEP_SHA256 = {
    "full-extraction": (
        "a74127e82bf70707c61eda011329960357621d355e2772fbdf05cafabc4c42d9",
        "499335b702b5d382cb1e2b4ba9ace58372d679960fd47f679b853d0f5ac10754",
        "506b3887881653c621022097239799dbfb212d08d81fd3cae960acca2ed0ade3",
        "5b9fed261d7079bc665babb015c4ac6546565d48541f85bb43f4fb3e1f4528e1",
        "8fd877a3faeabe35e2ee722ec75d0596f068c23df8d52363c5e091dfb564cbda",
        "7e4464193102973167e622b37d8a505c40f955e1a9c4a7fab8cc025a2dd51dec",
        "a60641b8aec1a0422204232a38b68facc744174dcf169ef4cfd2aa2a975f3a0f",
        "101ad2b3d3069169219616e1ef66c0de3c33cfe38c3ff53dbc88dc8601f0cb81",
    ),
    "heavy-benchmarks": (
        "a74127e82bf70707c61eda011329960357621d355e2772fbdf05cafabc4c42d9",
        "c6b8df90084fc9f6be59fa039c25ac554f06017aae901b97e83082eb9f58a755",
        "342a088e30eda3db746a5db0bed9bbfc69e729adf967c9e39157ad63179ff1cc",
        "128aca5d3603e2aa45761a5992c8292914e8ce50128645f8b1549022c098fa7c",
        "668d358b76ccc549d1acd666751f1c3304c0378b77412b334685a4de91753ca3",
        "1d34e83e1f2d4e08c1bf0394a2eccc8a0396a646459f1e713fe5f0f3b14b858f",
    ),
}
APPROVED_STEP_KINDS = {
    "full-extraction": (
        "checkout", "restore_cache", "restore_cache", "run", "run",
        "save_cache", "save_cache", "run",
    ),
    "heavy-benchmarks": (
        "checkout", "restore_cache", "run", "save_cache", "run", "store_artifacts",
    ),
}


def section(text: str, heading: str, indent: int) -> str:
    lines = text.splitlines()
    marker = " " * indent + heading + ":"
    matches = [i for i, line in enumerate(lines) if line == marker]
    if len(matches) != 1:
        raise AssertionError(f"expected one {marker!r}, found {len(matches)}")
    start = matches[0]
    end = next(
        (i for i in range(start + 1, len(lines))
         if lines[i].strip() and not lines[i].startswith(" " * (indent + 1))),
        len(lines),
    )
    return "\n".join(lines[start:end])


def mapping_errors(text: str, indent: int, allowed: set[str]) -> list[str]:
    """Check the owned YAML envelope; quoted/merged/duplicate keys fail closed."""
    errors: list[str] = []
    keys: list[str] = []
    for line in text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if len(line) - len(line.lstrip()) == indent:
            key = line.strip().split(":", 1)[0]
            keys.append(key)
            if key not in allowed:
                errors.append(f"unapproved execution property: {key}")
    if len(keys) != len(set(keys)):
        errors.append("duplicate execution property")
    return errors


def execution_envelope_errors(text: str, jobs: tuple[str, ...]) -> list[str]:
    errors = mapping_errors(text, 0, {"version", "parameters", "jobs", "workflows"})
    errors.extend(mapping_errors(section(text, "jobs", 0), 2, {"full-extraction", "heavy-benchmarks"}))
    errors.extend(mapping_errors(section(text, "workflows", 0), 2, {"normal", "heavy"}))
    for job in jobs:
        errors.extend(mapping_errors(job, 4, {"machine", "resource_class", "environment", "steps"}))
        errors.extend(mapping_errors(section(job, "machine", 4), 6, {"image"}))
        errors.extend(mapping_errors(section(job, "environment", 4), 6, {"CARGO_BUILD_JOBS"}))
        lines = job.splitlines()
        for index, line in enumerate(lines):
            if line == "      - run:":
                end = next((i for i in range(index + 1, len(lines))
                            if lines[i].strip() and len(lines[i]) - len(lines[i].lstrip()) <= 6), len(lines))
                errors.extend(mapping_errors("\n".join(lines[index + 1:end]), 10,
                                             {"name", "no_output_timeout", "command"}))
    return errors


def step_blocks(job: str) -> list[str]:
    lines = section(job, "steps", 4).splitlines()[1:]
    blocks: list[list[str]] = []
    for line in lines:
        if line.startswith("      - "):
            blocks.append([])
        if not blocks:
            if line.strip():
                raise AssertionError("content precedes first approved job step")
            continue
        blocks[-1].append(line)
    return ["\n".join(block).rstrip() + "\n" for block in blocks]


def approved_step_errors(name: str, job: str) -> list[str]:
    blocks = step_blocks(job)
    expected_kinds = APPROVED_STEP_KINDS[name]
    kinds = tuple(block.splitlines()[0].strip()[2:].split(":", 1)[0] for block in blocks)
    if kinds != expected_kinds:
        return [f"{name} step order changed: {kinds}"]
    expected_hashes = APPROVED_STEP_SHA256[name]
    if len(blocks) != len(expected_hashes):
        return [f"{name} approved step count changed"]
    return [
        f"{name} step {index + 1} changed; review its full command/cache contract"
        for index, (block, expected) in enumerate(zip(blocks, expected_hashes))
        if hashlib.sha256(block.encode("utf-8")).hexdigest() != expected
    ]


def config_errors(text: str) -> list[str]:
    errors: list[str] = []
    try:
        params = section(text, "parameters", 0)
        normal = section(section(text, "workflows", 0), "normal", 2)
        heavy_flow = section(section(text, "workflows", 0), "heavy", 2)
        full = section(section(text, "jobs", 0), "full-extraction", 2)
        heavy = section(section(text, "jobs", 0), "heavy-benchmarks", 2)
    except AssertionError as error:
        return [str(error)]
    try:
        errors.extend(execution_envelope_errors(text, (full, heavy)))
        errors.extend(approved_step_errors("full-extraction", full))
        errors.extend(approved_step_errors("heavy-benchmarks", heavy))
    except AssertionError as error:
        errors.append(str(error))
    if params.strip() != "parameters:\n  run_heavy:\n    type: boolean\n    default: false":
        errors.append("heavy parameter must be boolean and default false")
    if normal != "  normal:\n    unless: << pipeline.parameters.run_heavy >>\n    jobs:\n      - full-extraction":
        errors.append("normal workflow must run the full gate only when heavy is false")
    if heavy_flow != "  heavy:\n    when: << pipeline.parameters.run_heavy >>\n    jobs:\n      - heavy-benchmarks":
        errors.append("heavy workflow must require explicit true")
    if "resource_class: medium" not in full or 'CARGO_BUILD_JOBS: "2"' not in full:
        errors.append("normal job must match two CPUs")
    if "resource_class: large" not in heavy or 'CARGO_BUILD_JOBS: "4"' not in heavy:
        errors.append("benchmark job must match four CPUs")
    # A successful halt skips all remaining steps, including the mandatory gate.
    # This job has no legitimate circleci-agent command. Reject its use rather
    # than attempting to interpret arbitrary shell quoting or indirection.
    if "circleci-agent" in text:
        errors.append("agent early termination can bypass required verification")
    for job in (full, heavy):
        if re.findall(r"(?m)^      image: (.+)$", job) != [IMAGE]:
            errors.append("executor image differs from the approved pin")
        if re.search(r"(?m)^    type: (?!build$)", job):
            errors.append("verification jobs must execute build steps")
        if "rustup toolchain install 1.92.0 --profile minimal" not in job:
            errors.append("each job must install the pinned toolchain")
        if "rustc --version | grep -F 'rustc 1.92.0 '" not in job:
            errors.append("each job must verify the selected toolchain")
    if tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"] != "1.92.0":
        errors.append("CI Rust version differs from rust-toolchain.toml")
    for contract in (
        "ripgrep=14.1.0-1 shellcheck=0.9.0-1",
        "--component clippy --component rustfmt",
        "pip install semgrep==1.157.0",
        "cargo install cargo-deny --version 0.19.7 --locked --force",
        "cargo-deny --version | grep -Fx 'cargo-deny 0.19.7'",
        "semgrep --version | grep -Fx '1.157.0'",
        "shellcheck --version | grep -Fx 'version: 0.9.0'",
        "python3 --version | grep -E '^Python 3\\.12\\.'",
    ):
        if contract not in full:
            errors.append(f"missing pinned tool contract: {contract}")
    if text.count(FULL_STEP) != 1 or not re.search(
        rf"(?m)^      - run:\n"
        rf"          name: {re.escape(FULL_STEP)}\n"
        rf"          no_output_timeout: 20m\n"
        rf"          command: \|\n"
        rf"            set -euo pipefail\n"
        rf"            {re.escape(SHA_CHECK)}\n"
        rf"            {re.escape(FULL_COMMAND)}\n"
        rf"            {re.escape(SHA_CHECK)}(?:\n|$)",
        full,
    ):
        errors.append("full gate must be a direct, strict, executable job step")
    if full.count(SHA_CHECK) != 2:
        errors.append("full verification must bind checkout SHA before and after the gate")
    if full.count(FULL_COMMAND) != 1 or "continue-on-error" in text or "when: on_fail" in text:
        errors.append("full gate may not be skipped or have ignored failures")
    if "- store_artifacts:\n          path: /tmp/qmp-benchmark\n          destination: qmp-scaling" not in heavy:
        errors.append("source-bound benchmark artifacts must be retained")
    if "for run in 1 2 3; do" not in heavy or "scripts/run-scaling-bench.sh --profile=in-memory-pressure" not in heavy:
        errors.append("three pressure receipts missing")
    if "scripts/run-scaling-bench.sh\n" not in heavy or "standard-full.jsonl" not in heavy:
        errors.append("standard full receipt missing")
    if any(token in text for token in ("schedule:", "cron:", "- target/", "store_test_results:")):
        errors.append("unexpected schedule or compiled-output cache")
    if text.count("cargo fetch --locked") != 2:
        errors.append("both jobs must fetch exact lock dependencies before saving")
    if text.count("{{ checksum \"Cargo.lock\" }}") != 4:
        errors.append("cache keys must include the lockfile")
    if text.count("- save_cache:") != 3 or text.count("- restore_cache:") != 3:
        errors.append("cache read/write shape changed")
    for match in re.finditer(r"(?m)^          command: \|\n((?:^            .*\n|^\n)*)", text):
        body = match.group(1)
        if not body.startswith("            set -euo pipefail\n"):
            errors.append("run step lacks strict shell failure handling")
    if "|| true" in text or "set +e" in text:
        errors.append("failure swallowing is forbidden")
    if (ROOT / ".github/workflows/ci.yml").exists() or (ROOT / ".circleci/benchmark.yml").exists():
        errors.append("legacy CI entry points remain")
    return errors


class CiEntryPointContracts(unittest.TestCase):
    def test_current_config_and_executable(self) -> None:
        self.assertEqual(config_errors(CONFIG.read_text()), [])
        script = EXAMPLES.read_text()
        self.assertIn("set -euo pipefail", script)
        self.assertIn('scripts/extract-to-standalone-repo.sh "${qmp_extract_dir}"', script)
        self.assertIn('scripts/check-workspace.sh', (ROOT / "scripts/extract-to-standalone-repo.sh").read_text())
        self.assertEqual(script.count('--manifest-path "${qmp_extract_dir}/Cargo.toml"'), 3)
        self.assertIn(
            'cargo run --locked --manifest-path "${qmp_extract_dir}/Cargo.toml" '
            '-p quanta-memory-platform-facade --features transcript --example transcript_session',
            script,
        )
        self.assertTrue(EXAMPLES.stat().st_mode & 0o111)

    def test_early_halt_and_floating_tools_are_rejected(self) -> None:
        text = CONFIG.read_text()
        for altered in (
            text.replace("      - checkout", "      - checkout\n      - run: circleci-agent step halt", 1),
            text.replace("      - checkout", "      - checkout\n      - run: circleci-agent \"step\" \"halt\"", 1),
            text.replace("ubuntu-2404:2026.05.1", "ubuntu-2404:current", 1),
            text.replace("ubuntu-2404:2026.05.1", "ubuntu-2404:latest", 1),
            text.replace("pip install semgrep==1.157.0", "pip install semgrep", 1),
            text.replace("cargo-deny --version | grep -Fx 'cargo-deny 0.19.7'", "echo unchecked", 1),
            text.replace("    machine:", "    type: no-op\n    machine:", 1),
        ):
            with self.subTest(altered=altered):
                self.assertTrue(config_errors(altered))

    def test_execution_envelope_cannot_bypass_required_gate(self) -> None:
        text = CONFIG.read_text()
        for label, altered in (
            ("step shell", text.replace(
                "  heavy-benchmarks:\n", "          shell: /bin/true\n\n  heavy-benchmarks:\n", 1)),
            ("background", text.replace(
                "  heavy-benchmarks:\n", "          background: true\n\n  heavy-benchmarks:\n", 1)),
            ("step environment", text.replace(
                "  heavy-benchmarks:\n", "          environment: {BASH_ENV: /tmp/skip}\n\n  heavy-benchmarks:\n", 1)),
            ("job shell", text.replace("    steps:\n", "    shell: /bin/true\n    steps:\n", 1)),
            ("job environment", text.replace(
                '      CARGO_BUILD_JOBS: "2"', '      CARGO_BUILD_JOBS: "2"\n      BASH_ENV: /tmp/skip', 1)),
            ("root defaults", text.replace("jobs:\n", "defaults: {shell: /bin/true}\njobs:\n", 1)),
            ("duplicate machine image", text.replace(
                "      image: ubuntu-2404:2026.05.1", "      image: ubuntu-2404:2026.05.1\n      image: ubuntu-2404:current", 1)),
        ):
            with self.subTest(label=label):
                self.assertTrue(config_errors(altered), label)

    def test_only_approved_steps_and_exact_pins_execute(self) -> None:
        text = CONFIG.read_text()
        for label, altered in (
            ("indirect halt", text.replace(
                "      - checkout\n",
                '      - checkout\n      - run:\n          name: Halt early\n'
                '          command: |\n            set -euo pipefail\n'
                '            agent=agent\n            command="circleci-$agent"\n'
                '            "$command" step halt\n', 1)),
            ("extra run", text.replace(
                "      - checkout\n",
                '      - checkout\n      - run:\n          name: Stop before gate\n'
                '          command: |\n            set -euo pipefail\n            exit 0\n', 1)),
            ("checkout mapping", text.replace(
                "      - checkout\n", "      - checkout:\n          path: /tmp/foreign\n", 1)),
            ("other pinned image", text.replace(IMAGE, "ubuntu-2404:2026.07.1", 1)),
            ("tool pin", text.replace("semgrep==1.157.0", "semgrep==1.158.0", 1)),
            ("cache key", text.replace("full-cargo-deny0.19.7", "full-cargo-deny0.20.0", 1)),
            ("extra workflow", text + "  alternate:\n    jobs:\n      - heavy-benchmarks\n"),
            ("conditional step", text.replace(
                "          name: Verify tool versions and locked downloads\n",
                "          name: Verify tool versions and locked downloads\n"
                "          when: on_fail\n", 1)),
        ):
            with self.subTest(label=label):
                self.assertTrue(config_errors(altered), label)

    def test_heavy_and_full_gate_tampering_fail(self) -> None:
        text = CONFIG.read_text()
        for before, after in (
            ("default: false", "default: true"),
            ("unless: << pipeline.parameters.run_heavy >>", "when: << pipeline.parameters.run_heavy >>"),
            ("- full-extraction", "- heavy-benchmarks"),
            (FULL_COMMAND, "echo skipped"),
            (SHA_CHECK, "echo unchecked revision"),
            ("set -euo pipefail", "set +e"),
            ("- store_artifacts:", "- ignored_artifacts:"),
            ("cargo fetch --locked", "cargo fetch"),
        ):
            with self.subTest(before=before):
                self.assertTrue(config_errors(text.replace(before, after, 1)))


if __name__ == "__main__":
    unittest.main()
