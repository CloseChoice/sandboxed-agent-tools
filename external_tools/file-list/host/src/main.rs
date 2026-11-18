use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{FsPerms, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Tool {
    component: String,
    permissions: Permissions,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Permissions {
    directory: String,
    file: String,
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    file: File,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn run(component: &Path, directory: &Path, file: &Path) -> wasmtime::Result<String> {
    let engine = Engine::default();
    let component = Component::from_file(&engine, component)?;
    let mut linker = Linker::<State>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    linker.root().func_wrap(
        "read-file",
        |mut store: wasmtime::StoreContextMut<'_, State>, (): ()| {
            let mut text = String::new();
            let file = &mut store.data_mut().file;
            let result = file
                .rewind()
                .and_then(|_| file.read_to_string(&mut text))
                .map(|_| text)
                .map_err(|_| "could not read the selected UTF-8 file".to_string());
            Ok((result,))
        },
    )?;

    let mut wasi = WasiCtxBuilder::new();
    wasi.preopened_dir(directory, "/listing", FsPerms::ReadOnly)?;
    wasi.allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false);
    // No environment, stdio, or other filesystem access is inherited.
    let mut store = Store::new(
        &engine,
        State {
            wasi: wasi.build(),
            table: ResourceTable::new(),
            file: File::open(file)?,
        },
    );
    let instance = linker.instantiate(&mut store, &component)?;
    let run = instance.get_typed_func::<(String,), (Result<String, String>,)>(&mut store, "run")?;
    let name = file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let (result,) = run.call(&mut store, (name,))?;
    result.map_err(wasmtime::Error::msg)
}

fn main() -> wasmtime::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        wasmtime::bail!("usage: file-list-host TOOL.toml");
    }
    let config = Path::new(&args[0]).canonicalize()?;
    let tool: Tool = toml::from_str(&std::fs::read_to_string(&config)?)?;
    let base = config.parent().unwrap();
    print!(
        "{}",
        run(
            &base.join(tool.component),
            &base.join(tool.permissions.directory),
            &base.join(tool.permissions.file)
        )?
    );
    Ok(())
}
