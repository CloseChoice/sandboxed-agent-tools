#[path = "../examples/support/mod.rs"]
mod support;

use neo_frizbee::{Config, Matcher, Scoring, UnicodeMatching};

fn strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|p| (*p).into()).collect()
}

#[test]
fn wasm_matches_native_scores_and_order() -> wasmtime::Result<()> {
    let paths = strings(&[
        "src/main.rs", "docs/readme.md", "schema.rs", "src/schema.rs",
        "📁/café.rs", "src/Main.rs", "src/main.rs", "", "src\\main.rs",
    ]);
    let (mut store, bindings) = support::instantiate()?;
    let api = bindings.local_fff_fuzzy_search();
    let finder = api.finder().call_constructor(&mut store)?;
    api.finder().call_set_candidates(&mut store, finder, &paths)?.unwrap();
    for query in ["main", "smrs", "schexma", "café", "caf", "rs", "Main", "zzzzzz"] {
        for typos in 0..=2 {
            let uppercase = query.chars().any(char::is_uppercase);
            let config = Config {
                max_typos: Some(typos),
                unicode: UnicodeMatching::Always,
                scoring: Scoring {
                    capitalization_bonus: if uppercase { 8 } else { 0 },
                    matching_case_bonus: if uppercase { 4 } else { 0 },
                    ..Scoring::default()
                },
                ..Config::default()
            };
            let expected: Vec<_> = Matcher::new(query, &config).match_list(&paths)
                .iter().map(|h| (h.index, h.score)).collect();
            let hits = api.finder().call_search(&mut store, finder, query, 100, typos)?.unwrap();
            assert_eq!(hits.iter().map(|h| (h.id, h.score)).collect::<Vec<_>>(), expected,
                "query={query}, typos={typos}");
            // The export used by `wasmtime run --invoke` must agree as well.
            let single = api.call_match_paths(&mut store, &paths, query, 100, typos)?.unwrap();
            assert_eq!(single.iter().map(|h| (h.id, h.score)).collect::<Vec<_>>(), expected);
        }
    }
    finder.resource_drop(&mut store)?;
    Ok(())
}

#[test]
fn component_instantiates_without_host_imports() -> wasmtime::Result<()> {
    support::instantiate()?;
    Ok(())
}

#[test]
fn retains_candidates_ranks_duplicates_and_handles_empty_searches() -> wasmtime::Result<()> {
    let (mut store, bindings) = support::instantiate()?;
    let api = bindings.local_fff_fuzzy_search().finder();
    let finder = api.call_constructor(&mut store)?;
    api.call_set_candidates(&mut store, finder, &strings(&["docs/readme.md", "src/main.rs", "src/main.rs"]))?.unwrap();
    let hits = api.call_search(&mut store, finder, "main", 10, 0)?.unwrap();
    assert_eq!(hits.iter().map(|h| h.id).collect::<Vec<_>>(), [1, 2]);
    assert!(hits[0].score > 0);
    assert_eq!(hits[0].score, hits[1].score);
    assert_eq!(hits[0].highlights.iter().map(|h| (h.start, h.end)).collect::<Vec<_>>(), [(4, 8)]);
    assert_eq!(api.call_search(&mut store, finder, "main", 1, 0)?.unwrap().len(), 1);
    assert!(api.call_search(&mut store, finder, "main", 0, 0)?.unwrap().is_empty());
    assert!(api.call_search(&mut store, finder, "zzzzzz", 10, 0)?.unwrap().is_empty());
    let empty = api.call_search(&mut store, finder, "", 2, 0)?.unwrap();
    assert_eq!(empty.iter().map(|h| (h.id, h.score, h.highlights.len())).collect::<Vec<_>>(), [(0, 0, 0), (1, 0, 0)]);
    finder.resource_drop(&mut store)?;
    Ok(())
}

#[test]
fn fuzzy_subsequences_and_typo_budgets() -> wasmtime::Result<()> {
    let (mut store, bindings) = support::instantiate()?;
    let api = bindings.local_fff_fuzzy_search().finder();
    let finder = api.call_constructor(&mut store)?;
    api.call_set_candidates(&mut store, finder, &strings(&["src/main.rs", "schema.rs"]))?.unwrap();
    assert_eq!(api.call_search(&mut store, finder, "smrs", 10, 0)?.unwrap()[0].id, 0);
    assert!(api.call_search(&mut store, finder, "schexma", 10, 0)?.unwrap().is_empty());
    assert_eq!(api.call_search(&mut store, finder, "schexma", 10, 1)?.unwrap()[0].id, 1);
    finder.resource_drop(&mut store)?;
    Ok(())
}

#[test]
fn highlights_are_utf8_byte_ranges() -> wasmtime::Result<()> {
    let (mut store, bindings) = support::instantiate()?;
    let api = bindings.local_fff_fuzzy_search().finder();
    let finder = api.call_constructor(&mut store)?;
    let path = "📁/café.rs";
    api.call_set_candidates(&mut store, finder, &strings(&[path]))?.unwrap();
    for query in ["café", "caf", "é", "rs"] {
        let hits = api.call_search(&mut store, finder, query, 10, 0)?.unwrap();
        assert_eq!(hits.len(), 1);
        let highlighted: String = hits[0].highlights.iter()
            .map(|h| &path[h.start as usize..h.end as usize]).collect();
        assert_eq!(highlighted, query);
    }
    finder.resource_drop(&mut store)?;
    Ok(())
}

#[test]
fn resources_are_isolated_and_invalid_replacement_preserves_state() -> wasmtime::Result<()> {
    let (mut store, bindings) = support::instantiate()?;
    let api = bindings.local_fff_fuzzy_search().finder();
    let first = api.call_constructor(&mut store)?;
    let second = api.call_constructor(&mut store)?;
    api.call_set_candidates(&mut store, first, &strings(&["one"]))?.unwrap();
    api.call_set_candidates(&mut store, second, &strings(&["two"]))?.unwrap();
    assert!(api.call_search(&mut store, second, "one", 10, 0)?.unwrap().is_empty());
    assert!(api.call_set_candidates(&mut store, first, &["x".repeat(4097)])?.is_err());
    assert_eq!(api.call_search(&mut store, first, "one", 10, 0)?.unwrap().len(), 1);
    assert!(api.call_search(&mut store, first, &"x".repeat(257), 10, 0)?.is_err());
    assert!(api.call_search(&mut store, first, "one", 10, 3)?.is_err());
    api.call_set_candidates(&mut store, first, &[])?.unwrap();
    assert!(api.call_search(&mut store, first, "", 10, 0)?.unwrap().is_empty());
    assert_eq!(api.call_search(&mut store, second, "two", 10, 0)?.unwrap().len(), 1);
    api.call_set_candidates(&mut store, first, &strings(&["new"]))?.unwrap();
    assert_eq!(api.call_search(&mut store, first, "new", 10, 0)?.unwrap()[0].id, 0);
    first.resource_drop(&mut store)?;
    second.resource_drop(&mut store)?;
    Ok(())
}
