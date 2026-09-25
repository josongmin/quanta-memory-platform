import importlib.util
from pathlib import Path
import unittest


CHECKER = Path(__file__).resolve().parents[1] / "lib/check-extracted-metadata.py"
SPEC = importlib.util.spec_from_file_location("extraction_metadata", CHECKER)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ExtractionMetadataTests(unittest.TestCase):
    def setUp(self):
        self.root = Path("/tmp/qmp120-metadata-fixture")
        self.metadata = {
            "workspace_members": ["core", "facade", "testkit"],
            "packages": [
                {
                    "id": id_,
                    "name": name,
                    "source": None,
                    "manifest_path": str(self.root / "crates" / name / "Cargo.toml"),
                    "dependencies": [],
                }
                for id_, name in zip(
                    ["core", "facade", "testkit"], sorted(MODULE.EXPECTED_MEMBERS)
                )
            ],
        }

    def test_valid_three_member_workspace(self):
        MODULE.check_metadata(self.metadata, self.root)

    def test_external_host_path_dependency_rejected(self):
        self.metadata["packages"].append(
            {
                "id": "host",
                "name": "quanta-memory-host",
                "source": None,
                "manifest_path": "/tmp/semantica/quanta-memory-host/Cargo.toml",
            }
        )
        with self.assertRaisesRegex(ValueError, "forbidden host dependency"):
            MODULE.check_metadata(self.metadata, self.root)

    def test_optional_product_dependency_rejected_when_forbidden(self):
        self.metadata["packages"].append(
            {
                "id": "redb",
                "name": "redb",
                "source": "registry+https://example.invalid",
                "manifest_path": "/tmp/cargo-registry/redb/Cargo.toml",
            }
        )
        with self.assertRaisesRegex(ValueError, "forbidden selected dependency: redb"):
            MODULE.check_metadata(self.metadata, self.root, frozenset({"redb"}))

    def test_external_renamed_path_dependency_rejected(self):
        self.metadata["packages"].append(
            {
                "id": "external",
                "name": "unrelated-package",
                "source": None,
                "manifest_path": "/tmp/external/unrelated-package/Cargo.toml",
            }
        )
        with self.assertRaisesRegex(ValueError, "external path dependency"):
            MODULE.check_metadata(self.metadata, self.root)

    def test_unexpected_member_rejected(self):
        self.metadata["workspace_members"].append("extra")
        self.metadata["packages"].append(
            {
                "id": "extra",
                "name": "extra-package",
                "source": None,
                "manifest_path": str(self.root / "crates/extra-package/Cargo.toml"),
                "dependencies": [],
            }
        )
        with self.assertRaisesRegex(ValueError, "unexpected workspace members"):
            MODULE.check_metadata(self.metadata, self.root)

    def test_core_upward_dependency_is_rejected_from_resolved_graph(self):
        core = next(
            package
            for package in self.metadata["packages"]
            if package["name"] == "quanta-memory-platform-core"
        )
        core["dependencies"] = [
            {
                "name": "quanta-memory-platform-facade",
                "rename": "innocent_alias",
            }
        ]
        with self.assertRaisesRegex(
            ValueError,
            "quanta-memory-platform-core -> quanta-memory-platform-facade",
        ):
            MODULE.check_metadata(self.metadata, self.root)

    def test_facade_upward_dependency_is_rejected(self):
        facade = next(
            package
            for package in self.metadata["packages"]
            if package["name"] == "quanta-memory-platform-facade"
        )
        facade["dependencies"] = [{"name": "quanta-memory-platform-testkit"}]
        with self.assertRaisesRegex(
            ValueError,
            "quanta-memory-platform-facade -> quanta-memory-platform-testkit",
        ):
            MODULE.check_metadata(self.metadata, self.root)


if __name__ == "__main__":
    unittest.main()
