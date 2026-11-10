#!/usr/bin/env python3
"""Prepare, inspect, build, run, and evaluate the four-way challenge1 experiment."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import random
import shlex
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent
VARIANTS = ("baseline", "sat", "rtk", "sat_rtk")
TASK_ID = "psf__requests-2317"
BASE_COMMIT = "091991be0da19de9108dbe5e3752917fea3d7fdc"
RTK_SHA256 = "70fee7b4a548c43dde94c77e2335550981b0bc2fb013691d8c8109b5bd0eda37"


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def config():
    return json.loads((ROOT / "benchmark.json").read_text())


def resolve(path):
    return (ROOT / Path(path).expanduser()).resolve()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n")


def run(command, **kwargs):
    print("+ " + shlex.join(map(str, command)), flush=True)
    return subprocess.run(list(map(str, command)), check=True, **kwargs)


def prepare():
    """Local files only. Never calls a model, Docker, or the network."""
    task_dir = ROOT / "swe-bench-tasks" / "tasks" / TASK_ID
    metadata = (task_dir / "task.yaml").read_text()
    for expected in (f"base_commit: {BASE_COMMIT}", "- SWE-bench/SWE-bench_Lite", f"instance_id: {TASK_ID}"):
        if expected not in metadata:
            raise SystemExit(f"Unexpected task metadata: missing {expected!r}")
    task = {"instance_id": TASK_ID, "repo": "psf/requests", "base_commit": BASE_COMMIT,
            "dataset": "SWE-bench/SWE-bench_Lite", "split": "test",
            "problem_statement": (task_dir / "problem_statement.md").read_text(),
            "source_hashes": {n: digest(task_dir / n) for n in
                ("task.yaml", "problem_statement.md", "test.patch", "tests.json", "eval.sh")}}
    write_json(ROOT / "challenge1.json", task)
    print(f"Prepared {TASK_ID}. No containers or model calls started.")


def mounts(source, destination, readonly=True):
    # --mount rejects missing sources (unlike -v, which creates directories).
    return ["--mount", f"type=bind,src={source},dst={destination}" + (",readonly" if readonly else "")]


def container_command(variant, cfg, batch, name):
    folder = batch / variant
    command = ["docker", "run", "--rm", "--name", name, "--platform", "linux/amd64",
               "--user", f"{os.getuid()}:{os.getgid()}",
               "--cpus", str(cfg["cpus"]), "--memory", cfg["memory"],
               "--pids-limit", "256", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
               "--env", f"BENCH_VARIANT={variant}"]
    for key in cfg["credential_env"]:
        if key in os.environ:
            command += ["--env", key]  # Value inherited by Docker, not printed.
    command += mounts(batch / "run-config.json", "/config/benchmark.json")
    command += mounts(ROOT / "challenge1.json", "/config/challenge1.json")
    command += mounts(folder, "/results", readonly=False)
    if (batch / "models.json").exists():
        command += mounts(batch / "models.json", "/tmp/pi-agent/models.json")
    if variant in ("sat", "sat_rtk"):
        command += mounts(resolve(cfg["sat_binary"]), "/usr/local/bin/sat")
        for component in ("code", "ripgrep"):
            command += mounts(resolve(cfg["sat_registry"]) / component, f"/sat-registry/{component}")
    command += [f"sat-bench:{variant}"]
    return command


def build_commands(variants=VARIANTS):
    return [["docker", "build", "--platform", "linux/amd64", "--target", v,
             "--build-arg", f"BENCH_UID={os.getuid()}", "--build-arg", f"BENCH_GID={os.getgid()}",
             "--build-arg", f"RTK_SHA256={RTK_SHA256}", "--tag", f"sat-bench:{v}", str(ROOT)]
            for v in variants]


def preflight(cfg, variants=VARIANTS):
    if not cfg.get("model") or "/" not in cfg["model"]:
        raise SystemExit('Set "model" in benchmark.json or pass --model with an explicit "provider/model-id".')
    if not variants or len(set(variants)) != len(variants) or any(v not in VARIANTS for v in variants):
        raise SystemExit("Choose distinct supported experiment variants.")
    if cfg["timeout_seconds"] <= 0 or cfg["max_tool_calls"] <= 0:
        raise SystemExit("Time and tool-call limits must be positive.")
    if any(v in ("sat", "sat_rtk") for v in variants):
        sat_binary = resolve(cfg["sat_binary"])
        if not sat_binary.is_file() or not os.access(sat_binary, os.X_OK):
            raise SystemExit(f"Missing executable release SAT binary: {sat_binary}")
        registry = resolve(cfg["sat_registry"])
        for name in ("code", "ripgrep"):
            registered = json.loads((registry / name / "registration.json").read_text())
            if digest(registry / name / "tool.wasm") != registered["sha256"]:
                raise SystemExit(f"Integrity check failed for SAT {name}")
    for variant in variants:
        subprocess.run(["docker", "image", "inspect", f"sat-bench:{variant}"],
                       check=True, stdout=subprocess.DEVNULL)


def batch_variants(batch, variants=None):
    if variants is not None:
        return tuple(variants)
    manifest = batch / "manifest.json"
    return tuple(json.loads(manifest.read_text())["run_order"]) if manifest.exists() else VARIANTS


def collect(batch, variants=None):
    rows = []
    for variant in batch_variants(batch, variants):
        path = batch / variant / f"{variant}_challenge1.metrics.json"
        if not path.exists():
            rows.append({"variant": variant, "agent_completed": False, "error": "No metrics; see container log"})
            continue
        m = json.loads(path.read_text())
        rows.append({"variant": variant, **{k: m.get(k) for k in (
            "agent_wall_seconds", "container_wall_seconds", "agent_completed", "exit_code", "timed_out",
            "tool_calls", "tool_errors", "sat_calls", "rtk_rewritten_calls", "usage_complete",
            "official_swebench_resolved", "treatment_exercised", "blocked_tool_calls",
            "first_tool", "required_first_tool_succeeded")},
            **{f"tokens_{k}": v for k, v in m["tokens"].items()},
            "tool_counts": json.dumps(m["tool_counts"], sort_keys=True)})
    write_json(batch / "comparison.json", rows)
    keys = list(dict.fromkeys(k for row in rows for k in row))
    with (batch / "comparison.csv").open("w", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=keys)
        writer.writeheader()
        writer.writerows(rows)
    return rows


def execute(cfg, batch, variants=VARIANTS):
    preflight(cfg, variants)
    if batch.exists():
        raise SystemExit(f"Refusing to overwrite {batch}. Choose a new --results directory.")
    batch.mkdir(parents=True)
    os.chmod(batch, 0o700)
    # Only the chosen custom provider's model definitions are mounted, never auth.json.
    models = resolve(cfg["models_file"])
    if models.is_file():
        provider = cfg["model"].split("/", 1)[0]
        entry = json.loads(models.read_text()).get("providers", {}).get(provider)
        if entry is not None:
            write_json(batch / "models.json", {"providers": {provider: entry}})
            os.chmod(batch / "models.json", 0o600)
    write_json(batch / "run-config.json", {k: cfg[k] for k in
               ("model", "thinking", "timeout_seconds", "max_tool_calls")})
    order = list(variants)
    random.Random(cfg["seed"]).shuffle(order)
    manifest = {"config": cfg, "run_order": order, "task": json.loads((ROOT / "challenge1.json").read_text()),
                "sat_sha256": digest(resolve(cfg["sat_binary"])) if any(v in ("sat", "sat_rtk") for v in variants) else None,
                "rtk_archive_sha256": RTK_SHA256,
                "source_sha256": {n: digest(ROOT / n) for n in
                    ("Dockerfile", "package-lock.json", "agent-extension.ts", "container-run.py", "reporting.py", "tool_config.py", "task_prompts.py")},
                "images": {v: json.loads(subprocess.check_output(["docker", "image", "inspect", f"sat-bench:{v}"]))[0]["Id"]
                           for v in variants}}
    write_json(batch / "manifest.json", manifest)
    try:
        for variant in order:
            folder = batch / variant
            folder.mkdir(mode=0o700)
            name = f"sat-bench-{variant}-{os.getpid()}"
            command = container_command(variant, cfg, batch, name)
            start = time.perf_counter()
            try:
                with (folder / "container.log").open("w") as log:
                    result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT,
                                            timeout=cfg["timeout_seconds"] + 180)
                status = result.returncode
            except subprocess.TimeoutExpired:
                status = 124
            finally:
                # Ensure interruption/timeouts do not leave paid model calls running.
                subprocess.run(["docker", "rm", "--force", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            metrics_path = folder / f"{variant}_challenge1.metrics.json"
            if metrics_path.exists():
                metrics = json.loads(metrics_path.read_text())
                metrics["container_wall_seconds"] = time.perf_counter() - start
                metrics["container_exit_code"] = status
                write_json(metrics_path, metrics)
            print(f"{variant}: container exit {status}; logs: {folder}", flush=True)
    finally:
        # A selected custom provider config may contain a literal API key. Do not retain it in results.
        (batch / "models.json").unlink(missing_ok=True)
        rows = collect(batch)
    if any(not row.get("agent_completed") or row.get("required_first_tool_succeeded") is False for row in rows):
        raise SystemExit("One or more agent runs did not complete or skipped the required SAT call; inspect comparison.json and container logs.")


def evaluate(batch, variants=None):
    """Use the checked-out official harness, outside agent containers and timing."""
    interpreter = ROOT / ".venv/bin/python"
    harness = ROOT / ".venv/bin/swebench"
    if not interpreter.exists() or not harness.exists():
        raise SystemExit("Install the local SWE-bench harness in experiments/.venv first.")
    task_dir = ROOT / "swe-bench-tasks/tasks" / TASK_ID
    frozen = json.loads((ROOT / "challenge1.json").read_text())["source_hashes"]
    if any(digest(task_dir / n) != h for n, h in frozen.items()):
        raise SystemExit("Task evaluation files changed since prepare; refusing mismatched grading.")
    loader = ("import json,sys; from swebench.task.repo import load_task; "
              "print(json.dumps([load_task(sys.argv[1])]))")
    dataset = json.loads(subprocess.check_output([str(interpreter), "-c", loader, str(task_dir)], cwd=ROOT))
    dataset_file = batch / "evaluation-dataset.json"
    write_json(dataset_file, dataset)
    for variant in batch_variants(batch, variants):
        folder = batch / variant
        patch = folder / f"{variant}_challenge1.patch"
        if not patch.exists():
            continue
        prediction = folder / f"{variant}_challenge1.predictions.jsonl"
        prediction.write_text(json.dumps({"instance_id": TASK_ID, "model_name_or_path": variant,
                                         "model_patch": patch.read_text()}) + "\n")
        # Local dataset snapshot avoids downloading a dataset or rebuilding task images.
        # The official harness pulls task['image'] and creates a fresh evaluation container.
        run_id = "challenge1_" + hashlib.sha256(str(batch).encode()).hexdigest()[:10] + "_" + variant
        with (folder / "evaluation.log").open("w") as log:
            completed = subprocess.run([str(harness), "eval", str(dataset_file), "--predictions", str(prediction),
                "--instance", TASK_ID, "--run-id", run_id, "--workers", "1"],
                cwd=batch, stdout=log, stderr=subprocess.STDOUT)
        report_path = batch / "logs/evaluation" / run_id / variant / TASK_ID / "report.json"
        report_file = folder / f"{variant}_challenge1.evaluation.json"
        grade = {"harness_exit_code": completed.returncode, "resolved": None, "report": None}
        if report_path.exists():
            grade["report"] = json.loads(report_path.read_text())
            grade["resolved"] = grade["report"][TASK_ID].get("resolved")
        write_json(report_file, grade)
        metrics_path = folder / f"{variant}_challenge1.metrics.json"
        if metrics_path.exists():
            metrics = json.loads(metrics_path.read_text())
            metrics["official_swebench_resolved"] = grade["resolved"]
            write_json(metrics_path, metrics)
        print(f"{variant}: official resolved={grade['resolved']}; harness exit={completed.returncode}")
    collect(batch, variants)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "plan", "build", "run", "evaluate", "report", "doctor"))
    parser.add_argument("--results", default="results/challenge1")
    parser.add_argument("--variant", choices=VARIANTS, action="append",
                        help="Build/run only this variant; repeat to select several. Defaults to all configured variants.")
    parser.add_argument("--model", help="Override the provider/model-id in benchmark.json for this invocation")
    parser.add_argument("--execute", action="store_true", help="Required for model runs and official grading.")
    args = parser.parse_args()
    cfg = config()
    if args.model:
        cfg["model"] = args.model
    variants = tuple(args.variant or cfg["variants"])
    if len(set(variants)) != len(variants):
        parser.error("Do not repeat the same --variant.")
    cfg["variants"] = list(variants)
    batch = resolve(args.results)
    if args.action == "prepare":
        prepare()
    elif args.action == "plan":
        print("MODEL:", cfg["model"] or "UNSET — choose provider/model before running")
        for command in build_commands(variants):
            print(shlex.join(command))
        for v in variants:
            print(shlex.join(map(str, container_command(v, cfg, batch, f"sat-bench-{v}-PREVIEW"))))
        next_run = ["python3", "benchmark.py", "run"]
        for variant in variants:
            next_run += ["--variant", variant]
        if cfg["model"]:
            next_run += ["--model", cfg["model"]]
        next_run += ["--results", args.results, "--execute"]
        print("No commands executed. To run this selection:", shlex.join(next_run))
    elif args.action == "build":
        for command in build_commands(variants):
            run(command)
    elif args.action in ("run", "evaluate"):
        if not args.execute:
            raise SystemExit("No run started. Review `python3 benchmark.py plan`, then explicitly add --execute.")
        if args.action == "run":
            execute(cfg, batch, variants)
        else:
            evaluate(batch, args.variant)
    elif args.action == "report":
        print(json.dumps(collect(batch, args.variant), indent=2))
    elif args.action == "doctor":
        print("SAT binary:", resolve(cfg["sat_binary"]), resolve(cfg["sat_binary"]).exists())
        print("Model:", cfg["model"] or "UNSET")
        print("Package lock:", (ROOT / "package-lock.json").exists())
        subprocess.run(["docker", "version", "--format", "{{.Server.Version}}"], check=False)


if __name__ == "__main__":
    main()
