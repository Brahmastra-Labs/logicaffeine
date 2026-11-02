#!/usr/bin/env python3
"""Profile provenance must cover nested Rust modules used by the importer."""
import pathlib
import runpy
import tempfile
import unittest

class SourceCoverage(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        for name in [
            "Cargo.toml", "Cargo.lock",
            "crates/logicaffeine_kernel/Cargo.toml",
            "crates/logicaffeine_base/Cargo.toml",
            "crates/logicaffeine_kernel/src/lib.rs",
            "crates/logicaffeine_base/src/lib.rs",
            "crates/logicaffeine_kernel/examples/lean_arena.rs",
            "crates/logicaffeine_kernel/examples/lean_arena/dag.rs",
        ]:
            p = self.root / name
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text("// initial\n")
        scope = runpy.run_path(str(pathlib.Path(__file__).with_name("profile-lean-arena.py")))
        self.sources = scope["sources"]
        self.sources.__globals__["ROOT"] = self.root
        self.helper = "crates/logicaffeine_kernel/examples/lean_arena/dag.rs"

    def test_nested_importer_module_is_in_profile_provenance(self):
        self.assertIn(self.helper, self.sources())

    def test_changing_only_nested_module_invalidates_source_snapshot(self):
        before = self.sources()
        (self.root / self.helper).write_text("// changed implementation\n")
        self.assertNotEqual(before, self.sources())

if __name__ == "__main__":
    unittest.main()
