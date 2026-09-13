import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { search } from '../dist/js/fff-fuzzy.js';

test('Wasm scores and ordering agree with the native matcher', async () => {
  const expected = JSON.parse(await readFile(new URL('../target/native-reference.json', import.meta.url), 'utf8'));
  const finder = new search.Finder();
  finder.setCandidates([
    'src/main.rs', 'docs/readme.md', 'schema.rs', 'src/schema.rs',
    '📁/café.rs', 'src/Main.rs', 'src/main.rs', '', 'src\\main.rs',
  ]);
  let i = 0;
  for (const query of ['main', 'smrs', 'schexma', 'café', 'caf', 'rs', 'Main', 'zzzzzz']) {
    for (let typos = 0; typos <= 2; typos++) {
      assert.deepEqual(finder.search(query, 100, typos).map(h => [h.id, h.score]), expected[i++], `${query}, typos=${typos}`);
    }
  }
  finder[Symbol.dispose]();
});

test('component declares no host imports', async () => {
  const wit = await readFile(new URL('../dist/component.wit', import.meta.url), 'utf8');
  assert.doesNotMatch(wit, /\bimport\s/);
  assert.match(wit, /export local:fff-fuzzy\/search@0\.1\.0/);
});

test('retains candidates, ranks matches, and highlights paths', () => {
  const finder = new search.Finder();
  finder.setCandidates(['docs/readme.md', 'src/main.rs', 'src/main.rs']);
  const hits = finder.search('main', 10, 0);
  assert.deepEqual(hits.map(h => h.id), [1, 2]);
  assert.ok(hits[0].score > 0);
  assert.equal(hits[0].score, hits[1].score);
  assert.deepEqual(hits[0].highlights, [{ start: 4, end: 8 }]);
  assert.equal(finder.search('main', 1, 0).length, 1);
  assert.deepEqual(finder.search('main', 0, 0), []);
  assert.deepEqual(finder.search('zzzzzz', 10, 0), []);
  assert.deepEqual(finder.search('', 2, 0).map(h => [h.id, h.score, h.highlights]), [[0, 0, []], [1, 0, []]]);
  finder[Symbol.dispose]();
});

test('supports fuzzy subsequences and configurable typo tolerance', () => {
  const finder = new search.Finder();
  finder.setCandidates(['src/main.rs', 'schema.rs']);
  assert.equal(finder.search('smrs', 10, 0)[0].id, 0);
  assert.deepEqual(finder.search('schexma', 10, 0), []);
  assert.equal(finder.search('schexma', 10, 1)[0].id, 1);
  finder[Symbol.dispose]();
});

test('highlights are valid UTF-8 byte ranges, including ASCII after emoji', () => {
  const finder = new search.Finder();
  const path = '📁/café.rs';
  finder.setCandidates([path]);
  for (const query of ['café', 'caf', 'é', 'rs']) {
    const [hit] = finder.search(query, 10, 0);
    assert.ok(hit);
    const bytes = Buffer.from(path);
    const highlighted = hit.highlights.map(r => bytes.subarray(r.start, r.end).toString('utf8')).join('');
    assert.equal(highlighted, query);
  }
  finder[Symbol.dispose]();
});

test('resources are isolated and invalid replacement preserves state', () => {
  const first = new search.Finder();
  const second = new search.Finder();
  first.setCandidates(['one']);
  second.setCandidates(['two']);
  assert.deepEqual(second.search('one', 10, 0), []);
  assert.throws(() => first.setCandidates(['x'.repeat(4097)]));
  assert.equal(first.search('one', 10, 0).length, 1);
  assert.throws(() => first.search('x'.repeat(257), 10, 0));
  assert.throws(() => first.search('one', 10, 3));
  first.setCandidates([]);
  assert.deepEqual(first.search('', 10, 0), []);
  assert.equal(second.search('two', 10, 0).length, 1);
  first[Symbol.dispose]();
  second[Symbol.dispose]();
});
