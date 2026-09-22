//! MVP convention: named imported interfaces expose text read/write and directory list.
use crate::{metadata::Policy, runtime::State};
use anyhow::{Result, bail, ensure};
use cap_std::fs::Dir;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use wasmtime::component::{Component, Linker, Type, types::ComponentItem};
use wasmtime::{Engine, StoreContextMut};

#[derive(Clone, Debug, serde::Serialize)]
pub struct Capability {
    pub name: String,
    pub directory: bool,
    pub read: bool,
    pub write: bool,
}

pub fn discover(engine: &Engine, component: &Component) -> Result<Vec<Capability>> {
    let mut caps = Vec::new();
    for (name, item) in component.component_type().imports(engine) {
        if name.starts_with("wasi:") {
            continue;
        }
        let ComponentItem::ComponentInstance(interface) = item.ty else {
            bail!("unsupported import {name}");
        };
        let functions: Vec<_> = interface.exports(engine).collect();
        let directory = functions.iter().any(|(n, _)| *n == "list");
        let cap = Capability {
            name: name.into(),
            directory,
            read: functions.iter().any(|(n, _)| *n == "read"),
            write: functions.iter().any(|(n, _)| *n == "write"),
        };
        ensure!(
            cap.read || cap.write,
            "unsupported filesystem import {name}"
        );
        for (operation, item) in functions {
            ensure!(
                ["read", "write", "list"].contains(&operation),
                "unsupported operation {name}.{operation}"
            );
            let ComponentItem::ComponentFunc(f) = item.ty else {
                bail!("expected function {name}.{operation}");
            };
            let count = match operation {
                "list" => 1,
                "read" => usize::from(directory),
                "write" => 1 + usize::from(directory),
                _ => unreachable!(),
            };
            ensure!(
                !f.async_()
                    && f.params().len() == count
                    && f.params().all(|(_, t)| matches!(t, Type::String)),
                "invalid parameters for {name}.{operation}"
            );
            let result = match f.results().collect::<Vec<_>>().as_slice() {
                [Type::Result(r)] => r.clone(),
                _ => bail!("expected one result for {name}.{operation}"),
            };
            ensure!(
                matches!(result.err(), Some(Type::String)),
                "expected string error for {name}.{operation}"
            );
            let ok = match operation {
                "read" => matches!(result.ok(), Some(Type::String)),
                "write" => result.ok().is_none(),
                "list" => {
                    matches!(result.ok(), Some(Type::List(l)) if matches!(l.ty(), Type::String))
                }
                _ => false,
            };
            ensure!(ok, "invalid return type for {name}.{operation}");
        }
        caps.push(cap);
    }
    Ok(caps)
}

pub struct Binding {
    resource: Resource,
    read: bool,
    write: bool,
}
enum Resource {
    Directory(Dir),
    File(File),
}

pub fn open(cap: &Capability, path: &Path, policy: &Policy) -> Result<Binding> {
    let rules = if cap.directory {
        &policy.folders
    } else {
        &policy.files
    };
    let allows = |operation: &str| {
        rules.as_ref().is_none_or(|rules| {
            rules
                .iter()
                .any(|r| r.name == cap.name && r.access.iter().any(|s| s == operation))
        })
    };
    let read = cap.read && allows("read");
    let write = cap.write && allows("write");
    ensure!(read || write, "package policy denies binding {}", cap.name);
    let resource = if cap.directory {
        Resource::Directory(Dir::open_ambient_dir(path, cap_std::ambient_authority())?)
    } else {
        Resource::File(OpenOptions::new().read(read).write(write).open(path)?)
    };
    Ok(Binding {
        resource,
        read,
        write,
    })
}

fn safe_error<T>(r: std::io::Result<T>) -> Result<T, String> {
    // Do not disclose host paths in filesystem error messages.
    r.map_err(|e| format!("filesystem operation failed: {:?}", e.kind()))
}
impl Binding {
    fn read(&mut self, path: &str) -> Result<String, String> {
        if !self.read {
            return Err("read denied by package policy".into());
        }
        match &mut self.resource {
            Resource::Directory(dir) => safe_error(dir.read_to_string(path)),
            Resource::File(file) => safe_error((|| {
                file.seek(SeekFrom::Start(0))?;
                let mut text = String::new();
                file.read_to_string(&mut text)?;
                Ok(text)
            })()),
        }
    }
    fn write(&mut self, path: &str, text: &str) -> Result<(), String> {
        if !self.write {
            return Err("write denied by package policy".into());
        }
        match &mut self.resource {
            Resource::Directory(dir) => safe_error(dir.write(path, text)),
            Resource::File(file) => safe_error((|| {
                file.seek(SeekFrom::Start(0))?;
                file.set_len(0)?;
                file.write_all(text.as_bytes())
            })()),
        }
    }
    fn list(&self, path: &str) -> Result<Vec<String>, String> {
        if !self.read {
            return Err("list denied by package policy".into());
        }
        let Resource::Directory(dir) = &self.resource else {
            return Err("not a directory".into());
        };
        let mut names = safe_error(dir.read_dir(path).and_then(|entries| {
            entries
                .map(|entry| entry.map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect::<std::io::Result<Vec<_>>>()
        }))?;
        names.sort();
        Ok(names)
    }
}

pub fn link(linker: &mut Linker<State>, caps: &[Capability]) -> Result<()> {
    for cap in caps {
        let mut interface = linker.instance(&cap.name)?;
        if cap.read {
            let name = cap.name.clone();
            if cap.directory {
                interface.func_wrap(
                    "read",
                    move |mut s: StoreContextMut<'_, State>, (path,): (String,)| {
                        Ok((s.data_mut().files.get_mut(&name).unwrap().read(&path),))
                    },
                )?;
            } else {
                interface.func_wrap("read", move |mut s: StoreContextMut<'_, State>, (): ()| {
                    Ok((s.data_mut().files.get_mut(&name).unwrap().read(""),))
                })?;
            }
        }
        if cap.write {
            let name = cap.name.clone();
            if cap.directory {
                interface.func_wrap(
                    "write",
                    move |mut s: StoreContextMut<'_, State>, (path, text): (String, String)| {
                        Ok((s
                            .data_mut()
                            .files
                            .get_mut(&name)
                            .unwrap()
                            .write(&path, &text),))
                    },
                )?;
            } else {
                interface.func_wrap(
                    "write",
                    move |mut s: StoreContextMut<'_, State>, (text,): (String,)| {
                        Ok((s.data_mut().files.get_mut(&name).unwrap().write("", &text),))
                    },
                )?;
            }
        }
        if cap.directory {
            let name = cap.name.clone();
            interface.func_wrap(
                "list",
                move |s: StoreContextMut<'_, State>, (path,): (String,)| {
                    Ok((s.data().files[&name].list(&path),))
                },
            )?;
        }
    }
    Ok(())
}

pub type Bindings = BTreeMap<String, Binding>;
