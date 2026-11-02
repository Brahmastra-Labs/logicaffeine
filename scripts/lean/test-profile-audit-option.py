#!/usr/bin/env python3
"""Profiler dispatch/provenance controls; never run perf or a Rust build."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "profile_lean_arena", Path(__file__).with_name("profile-lean-arena.py"))
profiler = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profiler)


class AuditOptionTests(unittest.TestCase):
    def dispatch(self, flags):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "original input.ndjson"
            source.write_text('{"meta":{"format":{"version":"3.1.0"}}}\n')
            calls = []

            def run(command, **kwargs):
                calls.append(command)
                return subprocess.CompletedProcess(command, 0)

            with (
                patch.object(profiler, "ROOT", root),
                patch.object(profiler, "sources", return_value={"source.rs": "hash"}),
                patch.object(profiler, "digest", return_value="fixedhash"),
                patch.object(profiler, "render", return_value={}),
                patch.object(profiler.subprocess, "run", side_effect=run),
                patch.object(profiler.subprocess, "check_output", return_value="version"),
                patch.object(profiler.Path, "glob", return_value=iter(())),
                patch.object(sys, "argv", ["profile-lean-arena.py", *flags, str(source)]),
                contextlib.redirect_stdout(io.StringIO()),
            ):
                profiler.main()
            reports = list((root / "logs").glob("arena-profile-*/report.json"))
            self.assertEqual(len(reports), 1)
            report = json.loads(reports[0].read_text())
            command = report["command"]
            binary = str(root / "target/arena-profile/release/examples/lean_arena")
            checker = command[command.index(binary):]
            self.assertIn(command, calls, "provenance must describe the executed command")
            self.assertTrue(report["source_stable"])
            self.assertEqual(report["input_sha256"], "fixedhash")
            return checker, report, str(source)

    def test_default_keeps_independent_declaration_audit_disabled(self):
        checker, report, source = self.dispatch([])
        self.assertEqual(checker[1:], [source])
        self.assertIs(report["audit_declarations"], False)
        self.assertIs(report["trace_stages"], False)

    def test_explicit_audit_reaches_checker_and_provenance_without_forcing_trace(self):
        checker, report, source = self.dispatch(["--audit-declarations"])
        self.assertEqual(checker[1:], ["--audit-declarations", source])
        self.assertIs(report["audit_declarations"], True)
        self.assertIs(report["trace_stages"], False)

    def test_audit_and_trace_reach_the_same_checker_invocation(self):
        checker, report, source = self.dispatch(["--trace-stages", "--audit-declarations"])
        self.assertEqual(checker[1:], ["--trace-stages", "--audit-declarations", source])
        self.assertIs(report["audit_declarations"], True)
        self.assertIs(report["trace_stages"], True)


if __name__ == "__main__":
    unittest.main()
