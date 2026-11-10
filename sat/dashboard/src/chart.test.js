import test from 'node:test';
import assert from 'node:assert/strict';
import { slices, commandText } from './chart.js';

test('pie handles empty, single-tool, and mixed histories without degenerate full-circle arcs', () => {
  assert.deepEqual(slices([]), []);
  const single = slices([{ name: 'python', count: 10 }]);
  assert.equal(single[0].percent, 100);
  assert.equal((single[0].d.match(/ A /g) ?? []).length, 2);
  const mixed = slices([
    { name: 'python', count: 3 },
    { name: 'jaq', count: 1 },
  ]);
  assert.deepEqual(
    mixed.map((s) => s.percent),
    [75, 25],
  );
  assert(mixed.every((s) => !s.d.includes('NaN')));
});
test('command display preserves whitespace, empty values, quotes, and literal shell expansion', () => {
  assert.equal(
    commandText('python', ['-c', "print('hi')", '', '$(whoami)']),
    String.raw`sat python -c 'print('\''hi'\'')' '' '$(whoami)'`,
  );
});
