"""Report only completed message events, never streaming snapshots (double counting)."""
import collections
import json
from pathlib import Path


def read_jsonl(path):
    rows, malformed = [], 0
    if Path(path).exists():
        for line in Path(path).read_text().splitlines():
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                malformed += 1
    return rows, malformed


def report(prefix, runtime):
    prefix = str(prefix)
    events, malformed = read_jsonl(prefix + ".events.jsonl")
    tools, bad_tools = read_jsonl(prefix + ".tools.jsonl")
    messages = [e["message"] for e in events
                if e.get("type") == "message_end" and "message" in e]
    assistants = [m for m in messages if m.get("role") == "assistant"]
    usages = [m["usage"] for m in assistants if isinstance(m.get("usage"), dict)]
    fields = ["input", "output", "cacheRead", "cacheWrite", "totalTokens", "reasoning"]
    totals = {f: sum(u.get(f, 0) or 0 for u in usages)
              if any(f in u for u in usages) else None for f in fields}
    calls = [t for t in tools if t.get("type") == "tool_call"]
    results = [t for t in tools if t.get("type") == "tool_result"]
    first_tool = calls[0].get("toolName") if calls else None
    required_first = runtime.get("required_first_tool")
    first_succeeded = None if required_first is None else (
        first_tool == required_first and any(
            r.get("toolCallId") == calls[0].get("toolCallId") and not r.get("isError") for r in results))
    tool_counts = dict(collections.Counter(t["toolName"] for t in calls))
    sat_calls = sum(v for k, v in tool_counts.items() if k.startswith("sat_"))
    rtk_calls = sum(bool(t.get("rewritten")) for t in calls)
    stops = [m.get("stopReason") for m in assistants]
    metrics = {
        **runtime, "assistant_turns": len(assistants), "usage_messages": len(usages),
        "tokens": totals, "reported_cost": sum(u.get("cost", {}).get("total", 0) or 0 for u in usages),
        "tool_calls": len(calls), "tool_results": len(results), "tool_counts": tool_counts,
        "first_tool": first_tool, "required_first_tool_succeeded": first_succeeded,
        "tool_errors": sum(bool(t.get("isError")) for t in results),
        "rtk_rewritten_calls": rtk_calls, "sat_calls": sat_calls,
        "treatment_exercised": (runtime["variant"] not in ("sat", "sat_rtk") or sat_calls > 0)
            and (runtime["variant"] not in ("rtk", "sat_rtk") or rtk_calls > 0),
        "blocked_tool_calls": sum(bool(t.get("blocked")) for t in calls),
        "last_stop_reason": stops[-1] if stops else None,
        "agent_completed": runtime.get("exit_code") == 0 and bool(assistants)
            and stops[-1] == "stop" and len(calls) == len(results),
        "malformed_event_lines": malformed, "malformed_tool_lines": bad_tools,
        "usage_complete": bool(assistants) and len(usages) == len(assistants)
            and all("totalTokens" in u for u in usages),
    }
    Path(prefix + ".metrics.json").write_text(json.dumps(metrics, indent=2) + "\n")
    trace = [f"# {runtime['variant']} / challenge1\n",
             "Provider-visible messages and emitted reasoning, when supplied. "
             "Private internal reasoning and opaque signatures are not decoded.\n"]
    for m in messages:
        trace.append(f"\n## {m.get('role', 'message')}\n")
        content = m.get("content", [])
        if isinstance(content, str):
            trace.append(content + "\n")
            continue
        for b in content:
            kind = b.get("type")
            if kind == "text":
                trace.append(b.get("text", "") + "\n")
            elif kind == "thinking":
                trace.append("\n### Emitted reasoning\n" + b.get("thinking", "[unavailable]") + "\n")
            elif kind == "toolCall":
                trace.append(f"\n### Tool: {b.get('name')} ({b.get('id')})\n```json\n"
                             + json.dumps(b.get("arguments"), indent=2) + "\n```\n")
    trace.append("\n## Executed tools (including rewritten commands)\n")
    for t in calls:
        trace.append(f"\n- `{t['toolName']}` / `{t.get('toolCallId')}`"
                     f" / RTK rewrite: {bool(t.get('rewritten'))}\n\n```json\n"
                     + json.dumps({"requested": t.get("original"), "executed": t.get("input")}, indent=2)
                     + "\n```\n")
    Path(prefix + ".trace.md").write_text("\n".join(trace))
    return metrics
