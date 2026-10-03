import importlib.util
from pathlib import Path
import unittest


CHECKER = Path(__file__).resolve().parents[1] / "lib/check-extracted-metadata.py"
SPEC = importlib.util.spec_from_file_location("extraction_metadata", CHECKER)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)

CORE = "quanta-memory-platform-core"
FACADE = "quanta-memory-platform-facade"
TESTKIT = "quanta-memory-platform-testkit"


class ExtractionMetadataTests(unittest.TestCase):
    def setUp(self):
        self.root = Path("/tmp/qmp120-metadata-fixture")
        names = (CORE, FACADE, TESTKIT)
        self.metadata = {
            "workspace_root": str(self.root),
            "workspace_members": list(names),
            "packages": [
                {
                    "id": name,
                    "name": name,
                    "source": None,
                    "manifest_path": str(self.root / "crates" / name / "Cargo.toml"),
                    "dependencies": [],
                }
                for name in names
            ],
            "resolve": {"nodes": [{"id": name, "deps": []} for name in names]},
        }
        self.member(FACADE)["dependencies"] = [self.local_dependency(CORE)]
        self.node(FACADE)["deps"] = [
            {
                "name": "quanta_memory_platform_core",
                "pkg": CORE,
                "dep_kinds": [{"kind": None, "target": None}],
            }
        ]
        self.member(TESTKIT)["dependencies"] = [
            self.local_dependency(CORE),
            self.local_dependency(FACADE),
        ]
        self.node(TESTKIT)["deps"] = [
            {
                "name": "quanta_memory_platform_core",
                "pkg": CORE,
                "dep_kinds": [{"kind": None, "target": None}],
            },
            {
                "name": "quanta_memory_platform_facade",
                "pkg": FACADE,
                "dep_kinds": [{"kind": None, "target": None}],
            },
        ]

    def member(self, name):
        return next(package for package in self.metadata["packages"] if package["id"] == name)

    def node(self, name):
        return next(node for node in self.metadata["resolve"]["nodes"] if node["id"] == name)

    def local_dependency(self, name, **overrides):
        dependency = {
            "name": name,
            "source": None,
            "path": str(self.root / "crates" / name),
            "rename": None,
            "kind": None,
            "optional": False,
            "target": None,
        }
        dependency.update(overrides)
        return dependency

    def test_valid_three_member_workspace(self):
        MODULE.check_metadata(self.metadata, self.root)

    def test_renamed_required_dependency_keeps_local_identity(self):
        self.member(FACADE)["dependencies"][0]["rename"] = "core_alias"
        self.node(FACADE)["deps"][0]["name"] = "core_alias"
        MODULE.check_metadata(self.metadata, self.root)

    def test_missing_declared_downward_edges_rejected(self):
        for owner, target in ((FACADE, CORE), (TESTKIT, CORE), (TESTKIT, FACADE)):
            with self.subTest(owner=owner, target=target):
                dependencies = self.member(owner)["dependencies"]
                removed = next(dependency for dependency in dependencies if dependency["name"] == target)
                dependencies.remove(removed)
                with self.assertRaisesRegex(ValueError, "required normal dependency is missing"):
                    MODULE.check_metadata(self.metadata, self.root)
                dependencies.append(removed)

    def test_optional_dev_build_or_target_only_downward_edge_rejected(self):
        variants = (
            {"optional": True},
            {"kind": "dev"},
            {"kind": "build"},
            {"target": 'cfg(target_os = "none")'},
        )
        for changes in variants:
            with self.subTest(changes=changes):
                dependency = self.member(FACADE)["dependencies"][0]
                original = {key: dependency[key] for key in changes}
                dependency.update(changes)
                with self.assertRaisesRegex(ValueError, "required normal dependency is missing"):
                    MODULE.check_metadata(self.metadata, self.root)
                dependency.update(original)

    def test_missing_resolved_downward_edges_rejected(self):
        for owner, target in ((FACADE, CORE), (TESTKIT, CORE), (TESTKIT, FACADE)):
            with self.subTest(owner=owner, target=target):
                dependencies = self.node(owner)["deps"]
                removed = next(dependency for dependency in dependencies if dependency["pkg"] == target)
                dependencies.remove(removed)
                with self.assertRaisesRegex(ValueError, "required resolved dependency is missing"):
                    MODULE.check_metadata(self.metadata, self.root)
                dependencies.append(removed)

    def test_resolved_dev_or_target_only_edge_does_not_satisfy_normal_edge(self):
        variants = (
            {"kind": "dev", "target": None},
            {"kind": "build", "target": None},
            {"kind": None, "target": 'cfg(target_os = "none")'},
        )
        for kind in variants:
            with self.subTest(kind=kind):
                kinds = self.node(FACADE)["deps"][0]["dep_kinds"]
                original = kinds[0]
                kinds[0] = kind
                with self.assertRaisesRegex(ValueError, "required resolved dependency is missing"):
                    MODULE.check_metadata(self.metadata, self.root)
                kinds[0] = original

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

    def test_wrong_workspace_root_or_member_manifest_rejected(self):
        self.metadata["workspace_root"] = "/tmp/other-root"
        with self.assertRaisesRegex(ValueError, "metadata workspace root differs"):
            MODULE.check_metadata(self.metadata, self.root)
        self.metadata["workspace_root"] = str(self.root)
        self.member(CORE)["manifest_path"] = str(self.root / "other/core/Cargo.toml")
        with self.assertRaisesRegex(ValueError, "workspace member is not the local crate"):
            MODULE.check_metadata(self.metadata, self.root)

    def test_external_member_source_rejected(self):
        for source in ("registry+https://example.invalid", "git+https://example.invalid/qmp"):
            with self.subTest(source=source):
                self.member(CORE)["source"] = source
                with self.assertRaisesRegex(ValueError, "workspace member is not the local crate"):
                    MODULE.check_metadata(self.metadata, self.root)

    def test_external_same_name_package_rejected_by_package_id(self):
        for source in ("registry+https://example.invalid", "git+https://example.invalid/qmp"):
            with self.subTest(source=source):
                external = {
                    "id": f"{source}#{CORE}@0.2.0",
                    "name": CORE,
                    "source": source,
                    "manifest_path": "/tmp/external/quanta-memory-platform-core/Cargo.toml",
                    "dependencies": [],
                }
                self.metadata["packages"].append(external)
                with self.assertRaisesRegex(ValueError, "external duplicate of workspace member"):
                    MODULE.check_metadata(self.metadata, self.root)
                self.metadata["packages"].pop()

    def test_declared_member_dependency_must_point_to_local_path(self):
        variants = (
            {"path": None},
            {"path": "/tmp/elsewhere/quanta-memory-platform-core"},
            {"source": "registry+https://example.invalid", "path": None},
            {"source": "git+https://example.invalid/qmp", "path": None},
        )
        for changes in variants:
            with self.subTest(changes=changes):
                self.member(FACADE)["dependencies"] = [
                    self.local_dependency(CORE, rename="innocent_alias", **changes)
                ]
                with self.assertRaisesRegex(ValueError, "declared member dependency is not local"):
                    MODULE.check_metadata(self.metadata, self.root)

    def test_optional_dev_build_target_upward_rows_rejected_even_unresolved(self):
        variants = (
            {"optional": True},
            {"kind": "dev"},
            {"kind": "build"},
            {"target": "cfg(target_os = \"none\")"},
            {"rename": "innocent_alias"},
        )
        for changes in variants:
            with self.subTest(changes=changes):
                self.member(CORE)["dependencies"] = [self.local_dependency(FACADE, **changes)]
                with self.assertRaisesRegex(ValueError, "forbidden dependency direction"):
                    MODULE.check_metadata(self.metadata, self.root)

    def test_facade_upward_dependency_is_rejected(self):
        self.member(FACADE)["dependencies"] = [self.local_dependency(TESTKIT)]
        with self.assertRaisesRegex(ValueError, "forbidden dependency direction"):
            MODULE.check_metadata(self.metadata, self.root)

    def test_resolved_upward_edge_rejected_by_package_id_even_with_alias(self):
        self.node(CORE)["deps"] = [{"name": "innocent_alias", "pkg": FACADE}]
        with self.assertRaisesRegex(ValueError, "forbidden resolved dependency direction"):
            MODULE.check_metadata(self.metadata, self.root)

    def test_resolved_dependency_must_reference_known_package_id(self):
        self.node(FACADE)["deps"] = [{"name": "core_alias", "pkg": "missing"}]
        with self.assertRaisesRegex(ValueError, "resolved dependency package ID is missing"):
            MODULE.check_metadata(self.metadata, self.root)

    def test_missing_resolved_graph_rejected(self):
        self.metadata.pop("resolve")
        with self.assertRaisesRegex(ValueError, "resolved Cargo dependency graph is missing"):
            MODULE.check_metadata(self.metadata, self.root)


if __name__ == "__main__":
    unittest.main()
