mod support;

fn main() -> wasmtime::Result<()> {
    let mut args = std::env::args().skip(1);
    let query = args.next().unwrap_or_else(|| "main".into());
    let mut paths: Vec<String> = args.collect();
    if paths.is_empty() {
        paths = vec!["src/main.rs".into(), "docs/readme.md".into(), "src/schema.rs".into()];
    }

    let (mut store, bindings) = support::instantiate()?;
    let api = bindings.local_fff_fuzzy_search();
    let finder = api.finder().call_constructor(&mut store)?;
    api.finder().call_set_candidates(&mut store, finder, &paths)?
        .map_err(wasmtime::Error::msg)?;
    // Keep this resource to reuse the candidates for subsequent searches.
    let hits = api.finder().call_search(&mut store, finder, &query, 20, 0)?
        .map_err(wasmtime::Error::msg)?;
    for hit in hits {
        println!("{}: score={}, highlights={:?}", paths[hit.id as usize], hit.score, hit.highlights);
    }
    finder.resource_drop(&mut store)?;
    Ok(())
}
