// Exercise the real Pi SDK's tool registration and allowlisting, without prompting a model.
import assert from 'node:assert/strict';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { createAgentSession, DefaultResourceLoader, ModelRuntime, SessionManager, SettingsManager }
  from '@earendil-works/pi-coding-agent';
import { installBenchmark } from './agent-extension.ts';

const root = fileURLToPath(new URL('.', import.meta.url));
const exec = promisify(execFile);
const native = ['read', 'bash', 'edit', 'write', 'grep', 'find', 'ls'];
const satTools = ['sat_search', 'sat_outline', 'sat_read_symbol', 'sat_ripgrep'];

test('real Pi SDK exposes the shared runner allowlists and reproduces the old exclusion bug', async () => {
  const previous = process.env.BENCH_VARIANT;
  for (const [variant, reproduceBug] of [['baseline', false], ['sat', false], ['sat_rtk', false], ['sat', true]]) {
    const directory = mkdtempSync(join(tmpdir(), 'sat-pi-loadout-'));
    let session;
    try {
      process.env.BENCH_VARIANT = variant;
      const tools = reproduceBug ? native : JSON.parse((await exec('python3', ['-c',
        'import json,sys; from tool_config import active_tool_names; print(json.dumps(active_tool_names(sys.argv[1])))', variant],
        { cwd: root, encoding: 'utf8' })).stdout);
      const settingsManager = SettingsManager.inMemory({});
      const modelRuntime = await ModelRuntime.create({
        authPath: join(directory, 'auth.json'), modelsPath: null,
        modelsStorePath: join(directory, 'models-store.json'),
        refreshOnCreate: false, allowModelNetwork: false,
      });
      const loader = new DefaultResourceLoader({ cwd: directory, agentDir: directory,
        settingsManager, noExtensions: true, noSkills: true, noPromptTemplates: true,
        noThemes: true, noContextFiles: true,
        extensionFactories: [pi => installBenchmark(pi, { outputDir: directory, repo: directory })],
      });
      await loader.reload();
      assert.deepEqual(loader.getExtensions().errors, []);
      ({ session } = await createAgentSession({ cwd: directory, agentDir: directory, tools,
        settingsManager, modelRuntime, resourceLoader: loader,
        sessionManager: SessionManager.inMemory(directory),
      }));
      const errors = [];
      await session.bindExtensions({ onError: error => errors.push(error) });
      assert.deepEqual(errors, []);
      const active = session.getActiveToolNames();
      const expected = reproduceBug || variant === 'baseline' ? native : [...native, ...satTools];
      assert.deepEqual([...active].sort(), [...expected].sort());
      assert.deepEqual(session.agent.state.tools.map(t => t.name).sort(), [...expected].sort());
    } finally {
      session?.dispose();
      rmSync(directory, { recursive: true, force: true });
    }
  }
  if (previous === undefined) delete process.env.BENCH_VARIANT;
  else process.env.BENCH_VARIANT = previous;
});
