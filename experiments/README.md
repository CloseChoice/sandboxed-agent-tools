# SAT / RTK SWE-bench experiment

One SWE-bench Lite task, **`psf__requests-2317`**, run with Pi in four fresh
Docker containers. The issue concerns HTTP methods supplied as bytes.

| Variant | Native coding tools | SAT code retrieval / ripgrep | RTK shell rewrite |
| --- | --- | --- | --- |
| `baseline` (no SAT) | yes | no | no |
| `sat` | yes | yes | no |
| `rtk` | yes | no | yes |
| `sat_rtk` | yes | yes | yes |

One [Dockerfile](Dockerfile) has four named build targets. All share the same
task commit, Python 3.9.20, pytest 7.4.4, Pi 1.4.0, and native utilities.
RTK 1.2.0 is downloaded with a pinned SHA-256. SAT's existing **release binary**
and its `code`/`ripgrep` registry entries are mounted read-only at runtime into
the two SAT variants. Other SAT components, registry credentials, host source
directories, and the Docker socket are not mounted into agent containers.
The base image supplies a recent glibc compatible with the local SAT binary.
This initial setup targets Linux amd64.

Setup and tests do not run a benchmark. The model is deliberately unset in
[benchmark.json](benchmark.json). Choose an explicit `provider/model-id` and
use the same model and thinking level for every variant. Only the named
`credential_env` variables are forwarded; values never appear in the command
line. A custom provider definition, if needed, comes from the selected provider
in `~/.pi/agent/models.json`; `auth.json` is not mounted. Prefer environment
references for credentials. OAuth-only providers need additional configuration.

For exactly one SAT-enabled attempt directly on the host, see
[the local-run instructions](LOCAL_RUN.md). That path uses the same SAT wrappers,
captures traces/metrics, and requires an explicit `--execute` to contact the model.

To run just the SAT variant in Docker, from `experiments`:

```bash
python3 benchmark.py prepare
python3 benchmark.py build --variant sat

read -rsp 'Scaleway API key: ' SCW_SECRET_KEY
echo
export SCW_SECRET_KEY

python3 benchmark.py plan --variant sat \
  --model scaleway/deepseek-v4.1-flash --results results/docker-sat-challenge1

python3 benchmark.py run --variant sat \
  --model scaleway/deepseek-v4.1-flash --results results/docker-sat-challenge1 --execute
```

The build downloads dependencies and the clean task checkout; it makes no model
requests. Every new container starts from the original task commit in its image,
so changes in the earlier local checkout have no effect. SAT is mounted from the
host as configured above. Results are saved under
`results/docker-sat-challenge1/sat/`. Use a new results directory for each attempt.
The model override applies only to this invocation, leaving `benchmark.json` unchanged.

To grade that attempt with the official harness afterward:

```bash
python3 benchmark.py evaluate --results results/docker-sat-challenge1 --execute
python3 benchmark.py report --results results/docker-sat-challenge1
```

Grading runs in a separate official task container. Reports and grading infer
the selected variants from the run manifest. Without `--variant`, build/run
continue to use all four configured variants.

The downloaded harness, task repository, Python environments, credentials, and
run outputs are deliberately not checked in. To recreate the local evaluation
inputs from the `experiments` directory:

```sh
git clone https://github.com/SWE-bench/swe-bench-tasks.git swe-bench-tasks
git -C swe-bench-tasks checkout 3d07b464b7b311a0cbfb5ed5b2d8a3b96f84a33d
git clone https://github.com/SWE-bench/SWE-bench.git SWE-bench
git -C SWE-bench checkout 02e7a74ffd0b707aab73d203fe87bdc7c76afc8e
uv venv --python 3.14 .venv
uv pip install --python .venv/bin/python -e ./SWE-bench
```

The harness environment is separate from the old Requests task's Python 3.9.20
environment. These setup commands download dependencies but do not run a model.
`fuel-1trillion.patch` is retained as a historical record of an earlier experiment;
it is not applied by the runner. The current SAT runtime uses epoch deadlines
instead of fuel metering.

Prepare and inspect without calling a model or Docker:

```sh
python3 benchmark.py prepare
python3 benchmark.py plan
python3 -m unittest -v test_benchmark.py
```

The adapter tests use Node 24 and the pinned npm dependencies:

```sh
npm ci --ignore-scripts
npm test
```

After reviewing the plan, build the images (downloads dependencies; no model calls):

```sh
python3 benchmark.py build
```

Start the four runs only when ready:

```sh
python3 benchmark.py run --execute
```

Runs are sequential, in an order shuffled with seed 42. Each gets two CPUs,
4 GiB memory, 15 minutes, and an 80-tool-call budget. Containers run under the
calling user's UID/GID with all Linux capabilities dropped. Every container starts from
the pinned base commit with no previous run's session or files. SAT's component
cache starts cold inside each SAT container. No implicit global/project skills,
extensions, MCP tools, or AGENTS.md files are loaded.

SAT variants add four discoverable tools and instructions to prefer them for
retrieval; native tools remain available. RTK transparently runs `rtk rewrite`
on shell commands; commands without a rewrite run unchanged. RTK does not
compress Pi's native read/grep results or SAT's structured results. The
`treatment_exercised` field reports whether SAT was actually called and/or RTK
actually rewrote a command. These are measurements of the configured agent
workflows, rather than isolated microbenchmarks of the binaries.

Results are under `results/challenge1/`, with a separate directory per variant:

```text
comparison.csv / comparison.json / manifest.json
rtk/
  rtk_challenge1.trace.md          readable messages, emitted reasoning, tool calls/results
  rtk_challenge1.events.jsonl      raw Pi JSON event stream
  rtk_challenge1.session.jsonl     Pi session history
  rtk_challenge1.tools.jsonl       timestamped requested/executed commands and results
  rtk_challenge1.metrics.json      time, tokens, per-tool counts, errors, versions
  rtk_challenge1.prompt.txt        exact task prompt and variant guidance
  rtk_challenge1.patch             proposed solution, including new files
  rtk_challenge1.stderr.log
  artifacts/                      full native-tool output when Pi truncates it
```

The other variants use the same filenames with `baseline_`, `sat_`, and
`sat_rtk_` prefixes. Private internal thinking cannot be obtained. Provider-emitted
reasoning is preserved when present; opaque/redacted reasoning is not decoded.

`agent_wall_seconds` measures Pi startup through its exit, including inference,
tool execution, and logging. Image building, container startup, patch export, and
grading are excluded. `container_wall_seconds` is recorded separately. Tokens
come from completed assistant-message usage records, summed once per turn:
input, output, cache read/write, total, and reasoning when reported. Reasoning
tokens are already included in output tokens. Missing usage is `null`; it is
never silently treated as zero. Tool-call counts include attempts, with blocked
calls, errors, per-tool counts, and rewritten calls reported separately. RTK
internal subprocesses are not extra model tool calls.

Official correctness evaluation is a separate step:

```sh
python3 benchmark.py evaluate --execute
python3 benchmark.py report
```

This uses the existing local SWE-bench harness in `.venv`, creates a local
one-task dataset from `swe-bench-tasks`, and evaluates each patch in a fresh
official task image. The agent never receives `gold.patch`, `test.patch`, hints,
or the evaluation dataset. The official test patch is applied by the evaluator;
the designated FAIL_TO_PASS tests must pass and PASS_TO_PASS tests must retain
their required statuses. Results go to `*_challenge1.evaluation.json`, evaluation
logs, and the comparison's `official_swebench_resolved` column. An absent report
stays `null`, distinct from an evaluated failure. This old Requests task's test
suite uses HTTP endpoints, so endpoint failures can affect grading; inspect the
evaluation logs rather than attributing infrastructure failures to SAT/RTK.

Agent containers need network access to the model provider. Their shell is
container-isolated, not WASI-sandboxed; SAT's individual retrieval components get
only a read grant for `/testbed`. Instructions to avoid network/package installs
are experiment instructions, not a network security boundary. The runner does
not expose host filesystem authority beyond its explicit mounts.

A single task/run per variant validates the setup; it cannot establish a
statistical speed or token-efficiency advantage. Provider cache effects and
model nondeterminism still apply. Image IDs, config, SAT hash, dependency lock,
task/evaluation hashes, and run order are recorded for review. Existing result
directories are never overwritten: use `--results results/challenge1-repeat2`
for another batch.
