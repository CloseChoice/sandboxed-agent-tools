import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { componentNew, componentWit } from '@bytecodealliance/jco-transpile/wasm-tools';
import { generate, $init } from '@bytecodealliance/jco-transpile/component';

const core = await readFile('target/wasm32-unknown-unknown/release/fff_fuzzy_component.wasm');
const component = await componentNew(core, []);
const wit = await componentWit(component);
await $init;
const generated = generate(component, { name: 'fff-fuzzy' });
if (generated.imports.length || /^\s*import\s/m.test(wit)) {
  throw new Error(`Unexpected host imports: ${generated.imports.join(', ')}`);
}
await mkdir('dist', { recursive: true });
await writeFile('dist/fff-fuzzy.wasm', component);
await writeFile('dist/component.wit', wit);
for (const [name, bytes] of generated.files) {
  const output = join('dist/js', name);
  await mkdir(dirname(output), { recursive: true });
  await writeFile(output, bytes);
}
console.log(`Built dist/fff-fuzzy.wasm (${component.length} bytes; zero host imports)`);
