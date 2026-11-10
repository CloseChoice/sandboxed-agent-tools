"""Local-run planning must neither call a provider nor overwrite an existing run."""
from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("run_local", Path(__file__).with_name("run-local.py"))
run_local = importlib.util.module_from_spec(spec)
spec.loader.exec_module(run_local)


class LocalRunTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "node_modules/.bin").mkdir(parents=True)
        (self.root / "node_modules/.bin/pi").touch()
        sat = self.root / "sat"
        sat.touch()
        sat.chmod(0o700)
        (self.root / "benchmark.json").write_text(json.dumps({
            "sat_binary": "sat", "sat_registry": "registry", "thinking": "medium",
            "max_tool_calls": 80, "timeout_seconds": 900}))
        (self.root / "challenge1.json").write_text(json.dumps({
            "base_commit": "base", "instance_id": "challenge", "problem_statement": "Fix this issue"}))
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.results = self.root / "results"
        self.argv = ["run-local.py", "--repo", str(self.repo), "--results", str(self.results),
                     "--model", "provider/model"]

    def test_plan_does_not_start_model_or_create_results(self):
        with patch.object(run_local, "ROOT", self.root), patch("sys.argv", self.argv), \
                patch.object(run_local.subprocess, "check_output", side_effect=["base\n", ""]), \
                patch.object(run_local.subprocess, "Popen") as start, redirect_stdout(io.StringIO()) as output:
            run_local.main()
        start.assert_not_called()
        self.assertFalse(self.results.exists())
        self.assertIn("Plan only", output.getvalue())
        for tool in ("sat_search", "sat_outline", "sat_read_symbol", "sat_ripgrep"):
            self.assertIn(tool, output.getvalue())
        self.assertIn("very first tool call must be sat_ripgrep", output.getvalue())

    def test_existing_results_are_preserved(self):
        self.results.mkdir()
        marker = self.results / "existing.txt"
        marker.write_text("previous run")
        with patch.object(run_local, "ROOT", self.root), patch("sys.argv", self.argv + ["--execute"]), \
                patch.object(run_local.subprocess, "check_output", side_effect=["base\n", ""]), \
                patch.object(run_local.subprocess, "Popen") as start:
            with self.assertRaisesRegex(SystemExit, "already exists"):
                run_local.main()
        start.assert_not_called()
        self.assertEqual(marker.read_text(), "previous run")

    def test_dirty_checkout_is_rejected_before_execution(self):
        with patch.object(run_local, "ROOT", self.root), patch("sys.argv", self.argv + ["--execute"]), \
                patch.object(run_local.subprocess, "check_output", side_effect=["base\n", " M source.py\n"]), \
                patch.object(run_local.subprocess, "Popen") as start:
            with self.assertRaisesRegex(SystemExit, "clean checkout"):
                run_local.main()
        start.assert_not_called()


if __name__ == "__main__":
    unittest.main()
