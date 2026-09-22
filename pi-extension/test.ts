import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { discoverAndLoadExtensions } from "@earendil-works/pi-coding-agent";
import { searchTool, runSat } from "./sat.ts";

const cwd = fileURLToPath(new URL(".", import.meta.url));
const hit = { title: "Example", url: "https://example.com", description: "A search hit" };

test("Pi loads the local extension and registers web_search", async () => {
  const loaded = await discoverAndLoadExtensions([fileURLToPath(new URL("sat.ts", import.meta.url))], cwd,
    fileURLToPath(new URL("target/empty-agent-home", import.meta.url)));
  assert.deepEqual(loaded.errors, []);
  assert.ok(loaded.extensions.some(extension => extension.tools.has("web_search")));
});

test("queries stay in one argument, with fixed grants and propagated cancellation", async () => {
  const controller = new AbortController();
  const query = '--read-write=/ $(echo unsafe); "quoted"';
  const tool = searchTool(async (_binary, args, workdir, signal) => {
    assert.deepEqual(args, ["brave-search", `--query=${query}`, "--allow-http", "--json"]);
    assert.equal(workdir, cwd);
    assert.equal(signal, controller.signal);
    return JSON.stringify([hit]);
  });
  const result = await tool.execute("test", { query }, controller.signal, undefined, { cwd });
  assert.deepEqual(JSON.parse(result.content[0].text), [hit]);
});

test("results are bounded and empty or malformed results are handled", async () => {
  const invoke = (output: string) => searchTool(async () => output)
    .execute("test", { query: "hello" }, undefined, undefined, { cwd });
  const result = await invoke(JSON.stringify(Array(10).fill({ ...hit, description: "x".repeat(3000) })));
  assert.equal(result.details.results.length, 5);
  assert.equal(result.details.results[0].description.length, 1500);
  assert.equal((await invoke("[]")).content[0].text, "No search results.");
  await assert.rejects(invoke("not JSON"));
  await assert.rejects(invoke('[{"title":123}]'), /invalid search result/);
});

test("native runner executes without a shell and reports process failures", async () => {
  assert.equal(await runSat(process.execPath, ["-e", 'process.stdout.write("ok")'], cwd), "ok");
  await assert.rejects(runSat(process.execPath, ["-e", 'process.stderr.write("failure"); process.exit(1)'], cwd), /failure/);
  const controller = new AbortController();
  const pending = runSat(process.execPath, ["-e", "setInterval(() => {}, 1000)"], cwd, controller.signal);
  controller.abort();
  await assert.rejects(pending, /cancelled/);
});
