import { search } from '../dist/js/fff-fuzzy.js';

const [requestedQuery, ...suppliedPaths] = process.argv.slice(2);
const paths = suppliedPaths.length ? suppliedPaths
  : ['src/main.rs', 'src/schema.rs', 'docs/readme.md', 'tests/schema_test.rs'];
const finder = new search.Finder();
finder.setCandidates(paths);
for (const query of requestedQuery === undefined ? ['smrs', 'schexma'] : [requestedQuery]) {
  console.log(`Query: ${query}`);
  for (const hit of finder.search(query, 5, 1)) {
    console.log({ path: paths[hit.id], score: hit.score, highlights: hit.highlights });
  }
}
finder[Symbol.dispose]();
