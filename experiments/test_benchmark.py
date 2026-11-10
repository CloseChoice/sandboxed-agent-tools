"""Offline tests: accounting, absent usage, mounts, and explicit execution guard."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import benchmark
from reporting import report


class ReportingTests(unittest.TestCase):
    def test_required_first_sat_call_must_be_first_and_succeed(self):
        cases = [
            ([("sat_ripgrep", "sat")], [("sat", False)], True),
            ([("bash", "native"), ("sat_ripgrep", "sat")], [("native", False), ("sat", False)], False),
            ([("sat_ripgrep", "sat")], [("sat", True)], False),
            ([("sat_ripgrep", "sat")], [], False),
            ([], [], False),
        ]
        for calls, results, expected in cases:
            with self.subTest(calls=calls, results=results), tempfile.TemporaryDirectory() as tmp:
                prefix = Path(tmp) / "sat_challenge1"
                rows = [{"type": "tool_call", "toolName": name, "toolCallId": id} for name, id in calls]
                rows += [{"type": "tool_result", "toolCallId": id, "isError": error} for id, error in results]
                Path(str(prefix) + ".tools.jsonl").write_text("\n".join(map(json.dumps, rows)))
                metrics = report(prefix, {"variant": "sat", "exit_code": 0, "required_first_tool": "sat_ripgrep"})
                self.assertEqual(metrics["required_first_tool_succeeded"], expected)
                self.assertEqual(metrics["first_tool"], calls[0][0] if calls else None)

    def test_streaming_snapshots_do_not_inflate_usage(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / "rtk_challenge1"
            message = {"role": "assistant", "stopReason": "stop",
                       "content": [{"type": "thinking", "thinking": "A provider-visible summary"},
                                   {"type": "text", "text": "Done"}],
                       "usage": {"input": 10, "output": 4, "reasoning": 2,
                                 "cacheRead": 3, "cacheWrite": 0, "totalTokens": 17}}
            events = [{"type": "message_update", "message": message}] * 5
            events += [{"type": "message_end", "message": message},
                       {"type": "agent_end", "messages": [message]}]
            Path(str(prefix) + ".events.jsonl").write_text("\n".join(map(json.dumps, events)))
            tools = [{"type": "tool_call", "toolName": "bash", "toolCallId": "one",
                      "original": {"command": "git status"}, "input": {"command": "rtk git status"}, "rewritten": True},
                     {"type": "tool_result", "toolName": "bash", "toolCallId": "one", "isError": False}]
            Path(str(prefix) + ".tools.jsonl").write_text("\n".join(map(json.dumps, tools)))
            result = report(prefix, {"variant": "rtk", "exit_code": 0})
            self.assertEqual(result["tokens"]["totalTokens"], 17)
            self.assertEqual(result["tokens"]["output"], 4)  # Includes reasoning: do not add 2 again.
            self.assertEqual(result["tool_calls"], 1)
            self.assertEqual(result["rtk_rewritten_calls"], 1)
            self.assertTrue(result["agent_completed"])
            self.assertTrue(result["treatment_exercised"])
            trace = Path(str(prefix) + ".trace.md").read_text()
            self.assertIn("A provider-visible summary", trace)
            self.assertIn("rtk git status", trace)

    def test_crash_and_unknown_usage_are_not_reported_as_zero_token_success(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / "sat_challenge1"
            Path(str(prefix) + ".events.jsonl").write_text('{"type":"message_update"}\n{"partial":')
            result = report(prefix, {"variant": "sat", "exit_code": 124})
            self.assertIsNone(result["tokens"]["totalTokens"])
            self.assertFalse(result["agent_completed"])
            self.assertFalse(result["usage_complete"])
            self.assertFalse(result["treatment_exercised"])
            self.assertEqual(result["malformed_event_lines"], 1)

    def test_unfinished_tool_call_prevents_completed_status(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / "baseline_challenge1"
            Path(str(prefix) + ".events.jsonl").write_text(json.dumps({"type": "message_end", "message": {
                "role": "assistant", "stopReason": "stop", "content": []}}))
            Path(str(prefix) + ".tools.jsonl").write_text(json.dumps({"type": "tool_call", "toolName": "bash"}))
            self.assertFalse(report(prefix, {"variant": "baseline", "exit_code": 0})["agent_completed"])


class IsolationTests(unittest.TestCase):
    def test_single_variant_plan_does_not_launch_other_variants(self):
        result = subprocess.run([sys.executable, str(benchmark.ROOT / "benchmark.py"), "plan",
            "--variant", "sat", "--model", "provider/model"], capture_output=True, text=True, check=True)
        lines = result.stdout.splitlines()
        self.assertEqual(sum(line.startswith("docker build ") for line in lines), 1)
        self.assertEqual(sum(line.startswith("docker run ") for line in lines), 1)
        self.assertIn("--target sat ", result.stdout)
        self.assertIn("provider/model", result.stdout)
        self.assertNotIn("sat-bench:baseline", result.stdout)
        self.assertNotIn("sat-bench:rtk", result.stdout)
        self.assertNotIn("sat-bench:sat_rtk", result.stdout)

    def test_single_variant_preflight_requires_only_its_image(self):
        cfg = benchmark.config()
        cfg["model"] = "provider/model"
        with patch.object(benchmark.subprocess, "run") as inspect:
            benchmark.preflight(cfg, ("sat",))
        self.assertEqual(inspect.call_count, 1)
        self.assertEqual(inspect.call_args.args[0], ["docker", "image", "inspect", "sat-bench:sat"])

    def test_report_uses_manifest_variants(self):
        with tempfile.TemporaryDirectory() as tmp:
            batch = Path(tmp)
            (batch / "manifest.json").write_text(json.dumps({"run_order": ["sat"]}))
            folder = batch / "sat"
            folder.mkdir()
            (folder / "sat_challenge1.metrics.json").write_text(json.dumps({
                "agent_completed": True, "tokens": {"totalTokens": 10}, "tool_counts": {"sat_search": 1}}))
            rows = benchmark.collect(batch)
            self.assertEqual([row["variant"] for row in rows], ["sat"])
            self.assertTrue(rows[0]["agent_completed"])

    def test_only_sat_variants_mount_binary_and_components(self):
        cfg = benchmark.config()
        with tempfile.TemporaryDirectory() as tmp:
            batch = Path(tmp)
            for variant in benchmark.VARIANTS:
                cmd = benchmark.container_command(variant, cfg, batch, "test-preview")
                mount_args = [cmd[i + 1] for i, value in enumerate(cmd) if value == "--mount"]
                sat_mounts = [m for m in mount_args if "dst=/sat-registry/" in m or "dst=/usr/local/bin/sat" in m]
                self.assertEqual(len(sat_mounts), 3 if variant in ("sat", "sat_rtk") else 0)
                self.assertTrue(all(m.endswith(",readonly") for m in sat_mounts))
                result_mount = next(m for m in mount_args if "dst=/results" in m)
                self.assertIn(f"src={batch / variant},", result_mount)
                self.assertFalse(any("docker.sock" in m or "auth.json" in m or "gold.patch" in m for m in mount_args))

    def test_no_model_or_evaluation_run_without_execute(self):
        for action in ("run", "evaluate"):
            result = subprocess.run([sys.executable, str(benchmark.ROOT / "benchmark.py"), action],
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("No run started", result.stderr)


if __name__ == "__main__":
    unittest.main()
