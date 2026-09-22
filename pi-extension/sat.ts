import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { Type } from "@earendil-works/pi-ai";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const exec = promisify(execFile);

type Runner = (binary: string, args: string[], cwd: string, signal?: AbortSignal) => Promise<string>;

export const runSat: Runner = async (binary, args, cwd, signal) => {
  try {
    const result = await exec(binary, args, {
      cwd, signal, timeout: 45_000, maxBuffer: 256 * 1024,
      encoding: "utf8", windowsHide: true,
    });
    return result.stdout;
  } catch (error) {
    if (signal?.aborted) throw new Error("Search cancelled.");
    const failure = error as { code?: string; stderr?: string; killed?: boolean };
    if (failure.code === "ENOENT") throw new Error("SAT executable not found. Set SAT_BIN to its absolute path.");
    if (failure.killed) throw new Error("SAT exceeded its time or output limit.");
    throw new Error(failure.stderr?.trim().slice(0, 4000) || "SAT search failed.");
  }
};

export function searchTool(run: Runner = runSat) {
  return {
    name: "web_search",
    label: "Web search (SAT / Brave)",
    description: "Search the web with Brave through SAT. Returns up to five titles, URLs and snippets; does not fetch full pages.",
    parameters: Type.Object({
      query: Type.String({ minLength: 1, maxLength: 4096, description: "Web search query" }),
    }, { additionalProperties: false }),
    async execute(_id: string, params: { query: string }, signal: AbortSignal | undefined,
      _onUpdate: unknown, ctx: { cwd: string }) {
      // One argument array, no shell. The equals form keeps leading '-' in the
      // query from becoming a SAT option. The model cannot supply grant flags.
      const output = await run(process.env.SAT_BIN || "sat", [
        "brave-search", `--query=${params.query}`, "--allow-http", "--json",
      ], ctx.cwd, signal);
      const results: unknown = JSON.parse(output);
      if (!Array.isArray(results) || results.some(hit => !hit ||
        typeof hit.title !== "string" || typeof hit.url !== "string" || typeof hit.description !== "string")) {
        throw new Error("SAT returned an invalid search result.");
      }
      const hits = results.slice(0, 5).map(hit => ({
        title: hit.title.slice(0, 512), url: hit.url.slice(0, 2048), description: hit.description.slice(0, 1500),
      }));
      return {
        content: [{ type: "text" as const, text: hits.length ? JSON.stringify(hits) : "No search results." }],
        details: { results: hits },
      };
    },
  };
}

export default function (pi: ExtensionAPI) {
  pi.registerTool(searchTool());
}
