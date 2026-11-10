# One local Pi + SAT task attempt

This runs Pi directly on the host against **`psf__requests-2317`**, the same
SWE-bench Lite case as the Docker experiment. Pi receives the four `sat_*`
wrappers. Their components get read-only access to the task checkout. Pi's
native shell, editing tools, and test commands run with your host account's
permissions; this mode has no Docker isolation.

Run the following from the `experiments` directory. Setup downloads code and
dependencies but makes no model requests:

```bash
export SAT_EXPERIMENTS="$PWD"
npm ci --ignore-scripts
python3 benchmark.py prepare

mkdir -p "$SAT_EXPERIMENTS/.build/local-challenge1"
git init "$SAT_EXPERIMENTS/.build/local-challenge1/repo"
git -C "$SAT_EXPERIMENTS/.build/local-challenge1/repo" remote add origin \
  https://github.com/psf/requests.git
git -C "$SAT_EXPERIMENTS/.build/local-challenge1/repo" fetch --depth 1 origin \
  091991be0da19de9108dbe5e3752917fea3d7fdc
git -C "$SAT_EXPERIMENTS/.build/local-challenge1/repo" checkout --detach FETCH_HEAD
git -C "$SAT_EXPERIMENTS/.build/local-challenge1/repo" remote remove origin
```

The runner requires that commit and a clean working tree. It never resets files.
Use a fresh checkout for another attempt. No gold patch, evaluation test patch,
or hints are passed into the model's prompt. Host mode does not prevent a native
tool from reading other host paths; avoiding external solution research is a
prompt instruction here.

Prepare the task's native Python test environment. The existing experiments
`.venv` belongs to the evaluator and should not be used for this old Requests checkout:

```bash
uv venv --python 3.9.20 "$SAT_EXPERIMENTS/.build/local-challenge1/venv"
uv pip install \
  --python "$SAT_EXPERIMENTS/.build/local-challenge1/venv/bin/python" \
  'pytest==7.4.4' -e "$SAT_EXPERIMENTS/.build/local-challenge1/repo"
export PATH="$SAT_EXPERIMENTS/.build/local-challenge1/venv/bin:$PATH"
```

Pi uses its normal local configuration and credentials. For the existing
Scaleway provider in `~/.pi/agent/models.json`, enter the key locally:

```bash
read -rsp 'Scaleway API key: ' SCW_SECRET_KEY
echo
export SCW_SECRET_KEY
```

The key must be exported in the same shell that launches the runner. The selected
custom provider currently references `$SCW_SECRET_KEY`. No credentials need to
be placed into this repository. Local Pi can also use credentials saved through
its normal `/login` flow in `~/.pi/agent/auth.json`.

Inspect the invocation first; this does not contact the model or change the checkout:

```bash
python3 "$SAT_EXPERIMENTS/run-local.py" \
  --repo "$SAT_EXPERIMENTS/.build/local-challenge1/repo" \
  --model scaleway/deepseek-v4.1-flash
```

To start exactly **one** SAT-enabled attempt, add `--execute`:

```bash
python3 "$SAT_EXPERIMENTS/run-local.py" \
  --repo "$SAT_EXPERIMENTS/.build/local-challenge1/repo" \
  --model scaleway/deepseek-v4.1-flash \
  --execute
```

The runner launches pinned Pi 1.4.0 from this directory's npm dependencies,
sets the SAT binary/registry and repository/output paths, supplies the issue
and instructions, and stops after the model finishes. It inherits the task
Python environment from your shell. The model and thinking setting are explicit;
other global/project extensions, skills, MCP tools, and context files are disabled.
Native coding tools remain available alongside SAT. There is no dedicated web
search tool; outbound networking remains available to native host programs.

The explicit Pi tool allowlist includes the SAT names as well as native tools.
The extension writes a `tool_loadout` entry to the tool log listing the active
tools and any missing expected tools, so availability can be checked separately
from whether the model chose to call them.

The shared SAT usage guide includes examples and requires `sat_ripgrep` as the
first tool call, searching for `builtin_str` in `requests/sessions.py`. Skills
remain disabled; these instructions are appended directly to the system prompt.
Metrics record `first_tool` and `required_first_tool_succeeded`. Skipping or
failing the initial call causes the runner to report a protocol failure separately
from whether the agent finished its answer.

The run uses the 15-minute and 80-tool-call budgets in `benchmark.json`. SAT
components additionally retain their individual 30-second epoch deadline.
The existing host SAT compilation cache is reused.

Results are written to `results/local-challenge1/`:

- `sat_challenge1.trace.md`: readable messages, emitted reasoning, and tool calls.
- `sat_challenge1.events.jsonl` and `.session.jsonl`: Pi's raw event stream/session.
- `sat_challenge1.tools.jsonl`: requested and executed tools, results, and timings.
- `sat_challenge1.metrics.json`: wall time, reported token usage, and tool counts.
- `sat_challenge1.patch`: proposed solution, including new files.
- `sat_challenge1.prompt.txt` and `.stderr.log`.

An existing results directory is never overwritten. Use a new `--results` path
and a clean task checkout for a repeat. Proposed changes stay uncommitted in the
checkout. Private internal reasoning is unavailable; provider-emitted reasoning
is retained when supplied.

This produces a solution attempt, not an official SWE-bench grade. The current
Docker batch evaluation command expects the four-variant directory layout;
do not point it directly at this local results directory. The exported patch
can be graded separately with the official SWE-bench harness in a fresh task
container. Local agent test results and official benchmark resolution are
different measurements.
