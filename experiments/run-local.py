#!/usr/bin/env python3
"""Run one SAT-enabled Pi attempt on challenge1, directly on the host."""
import argparse
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import time

from reporting import report
from tool_config import active_tool_names
from task_prompts import REQUIRED_FIRST_SAT_TOOL, SAT_GUIDANCE

ROOT = Path(__file__).resolve().parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True, help="Fresh task checkout at its base commit")
    parser.add_argument("--model", required=True, help="Explicit provider/model-id")
    parser.add_argument("--results", type=Path, default=ROOT / "results/local-challenge1")
    parser.add_argument("--execute", action="store_true", help="Start the model run; otherwise print the plan")
    args = parser.parse_args()
    repo = args.repo.resolve()
    results = args.results.resolve()
    cfg = json.loads((ROOT / "benchmark.json").read_text())
    task = json.loads((ROOT / "challenge1.json").read_text())
    pi = ROOT / "node_modules/.bin/pi"
    if not pi.exists():
        raise SystemExit("Run npm ci --ignore-scripts in experiments first (installs pinned Pi).")
    sat = (ROOT / cfg["sat_binary"]).resolve()
    registry = (ROOT / cfg["sat_registry"]).resolve()
    if not sat.is_file() or not os.access(sat, os.X_OK):
        raise SystemExit("The release SAT executable is missing.")
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    status = subprocess.check_output(["git", "status", "--porcelain"], cwd=repo, text=True)
    if head != task["base_commit"] or status:
        raise SystemExit("Use a clean checkout at challenge1's base commit; the runner never resets your files.")
    if results.exists():
        raise SystemExit(f"Choose a new --results directory; {results} already exists.")
    prompt = (
        "Fix the following issue in the current repository. Inspect the code, implement the smallest correct fix, "
        "and run relevant tests. Do not use the network, install packages, or look up the original solution. "
        "Leave changes uncommitted and finish with a brief summary of the fix and tests.\n\n"
        + task["problem_statement"]
    )
    guidance = SAT_GUIDANCE
    prefix = results / "sat_challenge1"
    command = [str(pi), "--print", "--mode", "json", "--offline", "--no-extensions", "--no-mcp",
               "--no-skills", "--no-prompt-templates", "--no-themes", "--no-context-files", "--no-approve",
               "--extension", str(ROOT / "agent-extension.ts"), "--tools", ",".join(active_tool_names("sat")),
               "--session", str(prefix) + ".session.jsonl", "--model", args.model,
               "--thinking", cfg["thinking"], "--append-system-prompt", guidance, "--", prompt]
    print("Repository:", repo)
    print("Results:", results)
    print("Command:", shlex.join(command), flush=True)
    if not args.execute:
        print("Plan only. Add --execute to start one local model run.")
        return
    env = dict(os.environ, BENCH_VARIANT="sat", BENCH_REPO=str(repo), BENCH_OUTPUT_DIR=str(results),
               BENCH_MAX_TOOL_CALLS=str(cfg["max_tool_calls"]), SAT_BIN=str(sat), SAT_HOME=str(registry))
    # Host Pi reads its usual models.json/auth.json and inherited provider variables.
    results.mkdir(parents=True, mode=0o700)
    (results / "sat_challenge1.prompt.txt").write_text(prompt + "\n\n" + guidance + "\n")
    timed_out = False
    process = None
    start = time.perf_counter()
    try:
        with Path(str(prefix) + ".events.jsonl").open("w") as stdout, Path(str(prefix) + ".stderr.log").open("w") as stderr:
            process = subprocess.Popen(command, cwd=repo, env=env, stdout=stdout, stderr=stderr, start_new_session=True)
            try:
                code = process.wait(timeout=cfg["timeout_seconds"])
            except subprocess.TimeoutExpired:
                timed_out = True
                code = 124
    finally:
        if process is not None:
            # Stop the process group, including outstanding tool subprocesses.
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
    elapsed = time.perf_counter() - start
    # Include new files in the exported patch, without staging their contents or committing.
    subprocess.run(["git", "add", "-N", "."], cwd=repo, check=True)
    Path(str(prefix) + ".patch").write_bytes(subprocess.check_output(
        ["git", "diff", "--binary", task["base_commit"]], cwd=repo))
    metrics = report(prefix, {"variant": "sat", "execution": "local host; native tools unsandboxed",
        "instance_id": task["instance_id"], "model": args.model, "thinking": cfg["thinking"],
        "agent_wall_seconds": elapsed, "exit_code": code, "timed_out": timed_out,
        "official_swebench_resolved": None, "cache_policy": "existing host SAT cache reused",
        "required_first_tool": REQUIRED_FIRST_SAT_TOOL})
    print(json.dumps(metrics, indent=2))
    if not metrics["agent_completed"] or metrics["required_first_tool_succeeded"] is False:
        raise SystemExit("Run did not complete successfully or skipped the required initial SAT call; inspect the saved logs.")


if __name__ == "__main__":
    main()
