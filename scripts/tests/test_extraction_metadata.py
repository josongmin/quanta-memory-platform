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
            }
        )
        with self.assertRaisesRegex(ValueError, "unexpected workspace members"):
            MODULE.check_metadata(self.metadata, self.root)


if __name__ == "__main__":
    unittest.main()
