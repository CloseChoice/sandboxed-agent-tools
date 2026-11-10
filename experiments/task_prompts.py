"""SAT instructions shared by local and container experiments."""

REQUIRED_FIRST_SAT_TOOL = "sat_ripgrep"

SAT_GUIDANCE = '''SAT tool usage for this experiment

Your very first tool call must be sat_ripgrep with exactly these arguments:
{"query":"builtin_str","path":"requests/sessions.py","limit":10}
Call the registered tool directly, before any bash, read, grep, find, ls, or edit
call. This checks that the SAT path works on the issue's named file. Wait for its
result before continuing. If it fails, report the failure explicitly and diagnose
it before falling back; do not claim SAT worked without a successful result.

Available SAT calls:
- sat_ripgrep: query is a regular expression; returns matching lines.
  Example: {"query":"to_native_string","path":"requests","limit":10}
- sat_search: query is literal text; returns whole enclosing functions.
  Example: {"query":"builtin_str","path":"requests","limit":5}
- sat_outline: lists declarations/signatures in a file; omit query.
  Example: {"path":"requests/sessions.py","limit":20}
- sat_read_symbol: query is an exact symbol name; returns its definition.
  Example: {"query":"request","path":"requests/sessions.py","limit":5}

Paths are relative to the task repository. Do not use absolute paths or '..'.
Results use a virtual /src prefix: /src/requests/sessions.py corresponds to the
input path requests/sessions.py. The host grants read-only repository access;
you do not pass grant flags. Use small limits for previews and increase them
when needed. A truncated result is not the complete file or result set.

After the initial call, prefer SAT for code discovery and retrieval. Native tools
remain available for editing, tests, Git, and retrieval SAT cannot support.
'''
