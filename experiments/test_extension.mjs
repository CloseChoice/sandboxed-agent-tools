// Offline adapter tests. No provider, agent conversation, or Docker is started.
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { installBenchmark } from './agent-extension.ts';

function fixture(variant, run) {
  process.env.BENCH_VARIANT = variant;
  const outputDir = mkdtempSync(join(tmpdir(), 'sat-bench-test-'));
  const handlers = {}, tools = {};
  let active = [];
  const pi = {
    on: (name, handler) => { handlers[name] = handler; },
    registerTool: tool => { tools[tool.name] = tool; },
    setActiveTools: names => { active = names; },
    getActiveTools: () => active,
  };
  installBenchmark(pi, { outputDir, repo: '/fixture/repo', run });
  return { handlers, tools, active: () => active, outputDir,
    cleanup: () => rmSync(outputDir, { recursive: true, force: true }) };
}

test('baseline exposes no SAT tools, RTK rewrites only supported shell calls', async () => {
  const base = fixture('baseline', async () => { throw new Error('unexpected subprocess'); });
  try {
    await base.handlers.session_start();
    assert.equal(Object.keys(base.tools).length, 0);
    assert.equal(base.active().some(x => x.startsWith('sat_')), false);
  } finally { base.cleanup(); }
  const f = fixture('rtk', async (_bin, args) => {
    assert.equal(args[0], 'rewrite');
    if (args[1] === 'git status') return { stdout: 'rtk git status\n' };
    throw Object.assign(new Error('no rewrite'), { code: 1 });
  });
  try {
    const event = { type: 'tool_call', toolName: 'bash', toolCallId: '1', input: { command: 'git status' } };
    await f.handlers.tool_call(event);
    assert.equal(event.input.command, 'rtk git status');
    await f.handlers.tool_call({ type: 'tool_call', toolName: 'bash', toolCallId: '2', input: { command: 'python demo.py' } });
    const rows = readFileSync(join(f.outputDir, 'rtk_challenge1.tools.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
    assert.equal(rows[0].original.command, 'git status');
    assert.equal(rows[0].input.command, 'rtk git status');
    assert.equal(rows[1].input.command, 'python demo.py');
    assert.equal(rows[1].rewritten, false);
  } finally { f.cleanup(); }
});

test('SAT grants only the task repository and passes queries as literal arguments', async () => {
  const invocations = [];
  const f = fixture('sat_rtk', async (bin, args) => {
    invocations.push({ bin, args });
    return { stdout: 'matching function' };
  });
  try {
    await f.handlers.session_start();
    assert.equal(f.active().filter(x => x.startsWith('sat_')).length, 4);
    await f.tools.sat_search.execute('1', { query: '--read-write=/etc', path: 'requests/sessions.py' });
    const args = invocations[0].args;
    assert.equal(args[args.indexOf('--read') + 1], '/fixture/repo::/src');
    assert.ok(args.includes('/src/requests/sessions.py'));
    assert.ok(args.indexOf('--read-write=/etc') > args.indexOf('--'));
    for (const path of ['/etc/passwd', '../secret', 'requests/../../secret', '..\\secret']) {
      await assert.rejects(f.tools.sat_ripgrep.execute('2', { path, query: 'key' }));
    }
    assert.equal(invocations.length, 1);
  } finally { f.cleanup(); }
});

test('local runs use configured repository, log directory, and SAT/RTK binaries', async () => {
  const output = mkdtempSync(join(tmpdir(), 'sat-bench-local-'));
  const overrides = { BENCH_VARIANT: 'sat_rtk', BENCH_REPO: '/local/task', BENCH_OUTPUT_DIR: output,
    SAT_BIN: '/local/bin/sat', RTK_BIN: '/local/bin/rtk' };
  const previous = Object.fromEntries(Object.keys(overrides).map(k => [k, process.env[k]]));
  Object.assign(process.env, overrides);
  const tools = {}, handlers = {}, calls = [];
  try {
    installBenchmark({ on: (n, h) => { handlers[n] = h; },
      registerTool: t => { tools[t.name] = t; }, setActiveTools: () => {} }, {
      run: async (binary, args, options) => {
        calls.push({ binary, args, options });
        return { stdout: binary.endsWith('/rtk') ? 'rtk git status' : 'function example()' };
      },
    });
    await tools.sat_search.execute('local1', { query: 'example' });
    await handlers.tool_call({ type: 'tool_call', toolName: 'bash', toolCallId: 'local2', input: { command: 'git status' } });
    assert.equal(calls[0].binary, '/local/bin/sat');
    assert.equal(calls[0].options.cwd, '/local/task');
    assert.ok(calls[0].args.includes('/local/task::/src'));
    assert.equal(calls[1].binary, '/local/bin/rtk');
    assert.ok(readFileSync(join(output, 'sat_rtk_challenge1.tools.jsonl'), 'utf8').includes('sat_execution'));
  } finally {
    for (const [k, v] of Object.entries(previous)) {
      if (v === undefined) delete process.env[k]; else process.env[k] = v;
    }
    rmSync(output, { recursive: true, force: true });
  }
});
