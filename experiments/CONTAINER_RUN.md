# Run one SWE-bench task with Pi and SAT in Docker

These commands run one SAT-enabled attempt on `psf__requests-2317` from
SWE-bench Lite. Run them from the `experiments` directory with Docker running.

Prepare the task metadata and build the SAT variant image:

```bash
python3 benchmark.py prepare
python3 benchmark.py build --variant sat
```

The build downloads dependencies and a clean checkout at the task's original
commit. It makes no model requests. Changes in an earlier local task checkout
do not affect this container.

In the same terminal, supply your Scaleway key if it is not already exported:

```bash
read -rsp 'Scaleway API key: ' SCW_SECRET_KEY
echo
export SCW_SECRET_KEY
```

The runner passes the exported variable into Docker and mounts the selected
provider configuration from `~/.pi/agent/models.json`. That configuration must
define `scaleway/deepseek-v4.1-flash` and reference `$SCW_SECRET_KEY`.
The key is supplied at runtime, not stored in the image.

Preview the invocation without starting a container or contacting the model:

```bash
python3 benchmark.py plan \
  --variant sat \
  --model scaleway/deepseek-v4.1-flash \
  --results results/docker-sat-challenge1
```

Start one attempt:

```bash
python3 benchmark.py run \
  --variant sat \
  --model scaleway/deepseek-v4.1-flash \
  --results results/docker-sat-challenge1 \
  --execute
```

The container starts with a clean task checkout and mounts the optimized SAT
release binary and its `code` and `ripgrep` components read-only. Pi receives
`sat_search`, `sat_outline`, `sat_read_symbol`, and `sat_ripgrep`, alongside
native coding tools. SAT components retain their restricted filesystem grants
and epoch-based execution deadlines.

There is no dedicated web-search tool. Outbound networking remains available
for model requests and native shell commands; avoiding internet research is a
prompt instruction rather than an enforced network restriction.

Results go under:

```text
results/docker-sat-challenge1/
  comparison.csv
  comparison.json
  manifest.json
  sat/
    sat_challenge1.trace.md
    sat_challenge1.events.jsonl
    sat_challenge1.session.jsonl
    sat_challenge1.tools.jsonl
    sat_challenge1.metrics.json
    sat_challenge1.patch
    sat_challenge1.prompt.txt
    sat_challenge1.stderr.log
    container.log
```

The tool log includes a `tool_loadout` entry showing which tools were exposed.
Check `sat_calls` and `treatment_exercised` in the metrics to verify whether the
model actually used SAT. Traces retain provider-emitted reasoning when supplied;
private internal reasoning is unavailable.

The SAT variants receive a shared usage guide with argument examples and a required
first call: `sat_ripgrep` searches for `builtin_str` in `requests/sessions.py`.
Skills discovery remains disabled; the guide is appended directly to the system
prompt. `first_tool` and `required_first_tool_succeeded` record compliance. A run
that skips or fails the initial SAT call is reported as a protocol failure, even
if the agent later produces a patch. Rebuild the image after changing this guide.

Existing result directories are never overwritten. For another attempt, choose
a new `--results` directory. Each new container starts from the original task
checkout automatically.

After the attempt, run official SWE-bench grading separately:

```bash
python3 benchmark.py evaluate \
  --results results/docker-sat-challenge1 \
  --execute
```

Grading applies the exported patch and the benchmark's test patch in a fresh
official task container. It requires no model credentials. Its runtime is
separate from agent execution timing. HTTP-dependent tests in this old Requests
task can be affected by endpoint or network failures; inspect `evaluation.log`
when grading fails.

Refresh and display the comparison afterward:

```bash
python3 benchmark.py report --results results/docker-sat-challenge1
```
