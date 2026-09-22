use std::{path::Path, sync::OnceLock};
use wasmtime::{Engine, Store, component::{Component, Linker}};

wasmtime::component::bindgen!({ path: "wit", world: "fuzzy-finder" });

/// Cache the compiled component; give each caller its own instance and store.
pub fn instantiate() -> wasmtime::Result<(Store<()>, FuzzyFinder)> {
    static COMPILED: OnceLock<(Engine, Component)> = OnceLock::new();
    if COMPILED.get().is_none() {
        let engine = Engine::default();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("dist/fff-fuzzy.wasm");
        let component = Component::from_file(&engine, &path)?;
        // No host functions or WASI capabilities are permitted.
        if component.component_type().imports(&engine).next().is_some() {
            return Err(wasmtime::Error::msg("component must have zero host imports"));
        }
        let _ = COMPILED.set((engine, component));
    }
    let (engine, component) = COMPILED.get().unwrap();
    let linker = Linker::<()>::new(engine);
    let mut store = Store::new(engine, ());
    let bindings = FuzzyFinder::instantiate(&mut store, component, &linker)?;
    Ok((store, bindings))
}
