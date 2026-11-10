#!/usr/bin/env python3
"""Inside one disposable container. No image building or task download in timing."""
import json
import os
from pathlib import Path
import signal
import subprocess
import time

from reporting import report
from tool_config import active_tool_names
from task_prompts import REQUIRED_FIRST_SAT_TOOL, SAT_GUIDANCE


def main():
    config = json.loads(Path("/config/benchmark.json").read_text())
    task = json.loads(Path("/config/challenge1.json").read_text())
    variant = os.environ["BENCH_VARIANT"]
    prefix = Path(f"/results/{variant}_challenge1")
    if prefix.with_suffix(".metrics.json").exists():
        raise SystemExit("Refusing to overwrite a previous run.")
    os.environ["BENCH_MAX_TOOL_CALLS"] = str(config["max_tool_calls"])
    has_sat = variant in ("sat", "sat_rtk")
    has_rtk = variant in ("rtk", "sat_rtk")
    for name, expected in (("sat", has_sat), ("rtk", has_rtk)):
        import shutil
        if bool(shutil.which(name)) != expected:
            raise SystemExit(f"Incorrect {name} availability for {variant}")
    prompt = (
        "Fix the following issue in the repository at /testbed. Inspect the code, "
        "implement the smallest correct fix, and run relevant tests. Do not use "
        "the network, install packages, or look up the original solution. "
        "Leave your changes uncommitted. Finish with a brief summary of the fix and tests.\n\n"
        + task["problem_statement"]
    )
    guidance = SAT_GUIDANCE if has_sat else ""
    # Explicit flags exclude global/project extensions, MCP, skills, and context files.
    command = ["pi", "--print", "--mode", "json", "--offline", "--no-extensions",
               "--no-mcp", "--no-skills", "--no-prompt-templates", "--no-themes",
               "--no-context-files", "--no-approve", "--extension",
               "/opt/benchmark/agent-extension.ts", "--tools", ",".join(active_tool_names(variant)),
               "--session", str(prefix) + ".session.jsonl", "--model", config["model"],
               "--thinking", config["thinking"]]
    if guidance:
        command += ["--append-system-prompt", guidance]
    command += ["--", prompt]
    Path(str(prefix) + ".prompt.txt").write_text(prompt + "\n\n" + guidance + "\n")
    versions = {"pi": subprocess.check_output(["pi", "--version"], text=True).strip(),
                "python": subprocess.check_output(["python", "--version"], text=True).strip()}
    if has_sat:
        versions["sat"] = subprocess.check_output(["sat", "--version"], text=True).strip()
    if has_rtk:
        versions["rtk"] = subprocess.check_output(["rtk", "--version"], text=True).strip()
    started = time.perf_counter()
    timed_out = False
    with open(str(prefix) + ".events.jsonl", "w") as stdout, open(str(prefix) + ".stderr.log", "w") as stderr:
        process = subprocess.Popen(command, cwd="/testbed", stdout=stdout, stderr=stderr, start_new_session=True)
        try:
            code = process.wait(timeout=config["timeout_seconds"])
        except subprocess.TimeoutExpired:
            timed_out = True
            os.killpg(process.pid, signal.SIGTERM)
            try:
                code = process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                code = process.wait()
    elapsed = time.perf_counter() - started
    # Include new files in patch export, without committing the agent's changes.
    subprocess.run(["git", "add", "-N", "."], cwd="/testbed", check=True)
    patch = subprocess.check_output(["git", "diff", "--binary", task["base_commit"]], cwd="/testbed")
    Path(str(prefix) + ".patch").write_bytes(patch)
    metrics = report(prefix, {"variant": variant, "instance_id": task["instance_id"],
        "base_commit": task["base_commit"], "model": config["model"], "thinking": config["thinking"],
        "agent_wall_seconds": elapsed, "exit_code": code, "timed_out": timed_out,
        "cache_policy": "fresh container; cold SAT component cache", "versions": versions,
        "official_swebench_resolved": None,
        "required_first_tool": REQUIRED_FIRST_SAT_TOOL if has_sat else None})
    print(json.dumps(metrics))
    return 0 if metrics["agent_completed"] and metrics["required_first_tool_succeeded"] is not False else 1


if __name__ == "__main__":
    raise SystemExit(main())
