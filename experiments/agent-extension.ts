import { appendFileSync } from "node:fs";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { Type } from "typebox";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const exec = promisify(execFile);
type Options = {
  outputDir?: string;
  repo?: string;
  satBinary?: string;
  rtkBinary?: string;
  run?: (binary: string, args: string[], options: object) => Promise<{ stdout: string }>;
};
export function installBenchmark(pi: ExtensionAPI, options: Options = {}) {
  const variant = process.env.BENCH_VARIANT!;
  const outputDir = options.outputDir || process.env.BENCH_OUTPUT_DIR || "/results";
  const repo = options.repo || process.env.BENCH_REPO || "/testbed";
  const run = options.run || exec;
  const useSat = ["sat", "sat_rtk"].includes(variant);
  const useRtk = ["rtk", "sat_rtk"].includes(variant);
  const prefix = `${outputDir}/${variant}_challenge1`;
  let calls = 0;
  const starts = new Map<string, number>();
  const record = (event: unknown) => appendFileSync(`${prefix}.tools.jsonl`, JSON.stringify({
    timestamp: new Date().toISOString(), monotonic_ms: performance.now(), ...event as object,
  }) + "\n");

  // RTK is applied to shell calls only, matching its command-rewrite integration.
  // Unsupported commands run unchanged; rewrite errors fail the invocation.
  pi.on("tool_call", async (event) => {
    starts.set(event.toolCallId, performance.now());
    const original = structuredClone(event.input);
    if (++calls > Number(process.env.BENCH_MAX_TOOL_CALLS || 80)) {
      record({ ...event, original, blocked: true });
      // Stop the agent too: merely returning blocked lets it loop forever.
      setTimeout(() => process.exit(75), 10);
      return { block: true, reason: "Benchmark tool-call budget exhausted." };
    }
    let rewritten = false;
    if (useRtk && event.toolName === "bash") {
      try {
        const result = await run(options.rtkBinary || process.env.RTK_BIN || "/usr/local/bin/rtk", ["rewrite", String(event.input.command)], {
          cwd: repo, timeout: 10_000, maxBuffer: 1024 * 1024,
        });
        if (result.stdout.trim()) {
          event.input.command = result.stdout.trim();
          rewritten = true;
        }
      } catch (error) {
        if ((error as { code?: number }).code !== 1) {
          record({ ...event, original, rewrite_error: String(error) });
          return { block: true, reason: "RTK rewrite failed; see tool log." };
        }
      }
    }
    record({ ...event, original, rewritten });
  });
  pi.on("tool_result", async (event) => {
    const started = starts.get(event.toolCallId);
    const fullPath = (event.details as { fullOutputPath?: string } | undefined)?.fullOutputPath;
    let raw_output_artifact: string | undefined;
    if (fullPath && existsSync(fullPath)) {
      mkdirSync(`${outputDir}/artifacts`, { recursive: true });
      raw_output_artifact = `artifacts/${event.toolCallId.replace(/[^a-zA-Z0-9_-]/g, "_")}.txt`;
      copyFileSync(fullPath, `${outputDir}/${raw_output_artifact}`);
    }
    record({ ...event, duration_ms: started === undefined ? null : performance.now() - started, raw_output_artifact });
    starts.delete(event.toolCallId);
  });

  if (useSat) {
    const path = Type.Optional(Type.String({ description: "Relative path inside the task repository. Defaults to its root." }));
    for (const operation of ["search", "outline", "read-symbol", "ripgrep"] as const) {
      const name = `sat_${operation.replaceAll("-", "_")}`;
      pi.registerTool({
        name, label: name,
        description: operation === "ripgrep"
          ? "Search repository text with sandboxed SAT ripgrep. Query is a regex. Returns matching lines."
          : `SAT Tree-sitter ${operation}: retrieve whole enclosing functions, outlines, or an exact named symbol. Query is literal text.`,
        parameters: Type.Object({ query: Type.Optional(Type.String()), path,
          limit: Type.Optional(Type.Integer({ minimum: 1, maximum: 100 })) }),
        async execute(_id, params, signal) {
          const relative = params.path || ".";
          if (relative.startsWith("/") || relative.split(/[\\/]/).includes("..")) {
            throw new Error("Use a relative repository path without '..'.");
          }
          // Positional queries may begin with '-', so use -- before positionals below.
          const grantOptions = ["--paths", `/src/${relative}`, "--read", `${repo}::/src`,
            "--max-results", String(params.limit || 20)];
          const actual = operation === "ripgrep"
            ? ["ripgrep", ...grantOptions, "--", params.query || ""]
            : ["code", ...grantOptions, "--", operation, ...(operation === "outline" ? [] : [params.query || ""])];
          const started = performance.now();
          try {
            const result = await run(options.satBinary || process.env.SAT_BIN || "/usr/local/bin/sat", actual, {
              cwd: repo, signal, timeout: 60_000, maxBuffer: 4 * 1024 * 1024,
            });
            record({ type: "sat_execution", tool: name, args: actual, duration_ms: performance.now() - started });
            return { content: [{ type: "text", text: result.stdout }], details: {} };
          } catch (error) {
            record({ type: "sat_execution", tool: name, args: actual, duration_ms: performance.now() - started, error: String(error) });
            throw error;
          }
        },
      });
    }
  }
  pi.on("session_start", async () => {
    const expected = ["read", "bash", "edit", "write", "grep", "find", "ls",
      ...(useSat ? ["sat_search", "sat_outline", "sat_read_symbol", "sat_ripgrep"] : [])];
    pi.setActiveTools(expected);
    const active = pi.getActiveTools();
    record({ type: "tool_loadout", variant, expected, active,
      missing: expected.filter(name => !active.includes(name)) });
  });
}

export default function (pi: ExtensionAPI) { installBenchmark(pi); }
