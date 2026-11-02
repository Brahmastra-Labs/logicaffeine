#!/usr/bin/env python3
"""Real subprocess regressions: resource timing survives a timed-out checker."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "profile_lean_arena", Path(__file__).with_name("profile-lean-arena.py"))
profiler = importlib.util.module_from_spec(spec)
spec.loader.exec_module(profiler)


class TimeoutTimingTests(unittest.TestCase):
    def measure(self, code, seconds):
        with tempfile.TemporaryDirectory() as directory:
            timing = Path(directory) / "timing.json"
            run = subprocess.run(
                profiler.measured_command([sys.executable, "-c", code], timing, seconds),
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=5)
            self.assertTrue(timing.exists(), "a bounded checker run must emit resource evidence")
            self.assertTrue(timing.read_text().strip(),
                            "timeout must preserve resource timing for the terminated checker")
            return run, json.loads(timing.read_text())

    def test_timeout_preserves_partial_cpu_and_memory_evidence(self):
        run, timing = self.measure("import time; time.sleep(10)", 0.1)
        self.assertEqual(run.returncode, 124)
        self.assertGreaterEqual(timing["wall_seconds"], 0.09)
        self.assertLess(timing["wall_seconds"], 5)
        self.assertGreater(timing["max_rss_kib"], 0)
        self.assertGreaterEqual(timing["user_seconds"], 0)
        self.assertGreaterEqual(timing["system_seconds"], 0)

    def test_completed_checker_preserves_verdict_exit_code_and_timing(self):
        run, timing = self.measure("print('REJECT'); raise SystemExit(1)", 2)
        self.assertEqual(run.returncode, 1)
        self.assertEqual(run.stdout.strip(), "REJECT")
        self.assertGreater(timing["max_rss_kib"], 0)


if __name__ == "__main__":
    unittest.main()
