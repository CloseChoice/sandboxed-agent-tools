//! MVP convention: named imported interfaces expose text read/write and directory list.

use crate::{
    access::{self, Access},
    runtime::State,
};
use anyhow::{Result, bail, ensure};
use cap_std::fs::Dir;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
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
    path: PathBuf,
    access: Access,
}

enum Resource {
    Directory(Dir),
    File(File),
}

pub fn open(
    cap: &Capability,
    text: &str,
    access: &Access,
    home: &Path,
    cwd: &Path,
) -> Result<Binding> {
    let path = access::path(text, home, cwd)?;
    let path = std::fs::canonicalize(path)?;
    // Binding a resource does not itself widen its granted authority.
    let read = cap.read && access.authorize(&path, false).is_ok();
    let write = cap.write && access.authorize(&path, true).is_ok();
    ensure!(read || write, "no matching grant for binding {}", cap.name);
    let root = access
        .roots
        .iter()
        .filter(|r| path == r.path || r.directory && path.starts_with(&r.path))
        .max_by_key(|r| r.path.components().count())
        .ok_or_else(|| anyhow::anyhow!("binding not granted"))?;
    let resource = if root.directory {
        let dir = Dir::open_ambient_dir(&root.path, cap_std::ambient_authority())?;
        let rel = path
            .strip_prefix(&root.path)?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non UTF-8 binding"))?;
        let (parent, name) = access::secure_parent(&dir, rel)?;
        if cap.directory {
            use cap_fs_ext::DirExt;
            Resource::Directory(parent.open_dir_nofollow(name)?)
        } else {
            Resource::File(access::open_file(&parent, &name, write, false)?)
        }
    } else {
        ensure!(!cap.directory, "binding requires directory");
        // Explicit file grants pin the selected inode without exposing its parent.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .open(&path)?;
        access::check_file(&file)?;
        Resource::File(file)
    };
    Ok(Binding {
        resource,
        read,
        write,
        path,
        access: access.clone(),
    })
}

fn safe_error<T>(r: std::io::Result<T>) -> Result<T, String> {
    // Do not disclose host paths in filesystem error messages.
    r.map_err(|e| format!("filesystem operation failed: {:?}", e.kind()))
}

impl Binding {
    fn checked_path(&self, path: &str, write: bool) -> Result<PathBuf, String> {
        let path = Path::new(path);
        if path.components().any(|c| {
            !matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        }) {
            return Err("filesystem access denied".into());
        }
        let host = self.path.join(path);
        self.access
            .authorize(&host, write)
            .map_err(|_| "filesystem access denied".to_string())?;
        Ok(host)
    }

    fn read(&mut self, path: &str) -> Result<String, String> {
        if !self.read {
            return Err("read denied".into());
        }
        self.checked_path(path, false)?;
        let mut read = |file: &mut File| {
            safe_error((|| {
                access::check_file(file)?;
                file.seek(SeekFrom::Start(0))?;
                let mut text = String::new();
                file.read_to_string(&mut text)?;
                Ok(text)
            })())
        };
        match &mut self.resource {
            Resource::Directory(dir) => {
                let (parent, name) = safe_error(access::secure_parent(dir, path))?;
                read(&mut safe_error(access::open_file(
                    &parent, &name, false, false,
                ))?)
            }
            Resource::File(file) => read(file),
        }
    }

    fn write(&mut self, path: &str, text: &str) -> Result<(), String> {
        if !self.write {
            return Err("write denied".into());
        }
        self.checked_path(path, true)?;
        let mut write = |file: &mut File| {
            safe_error((|| {
                access::check_file(file)?;
                file.seek(SeekFrom::Start(0))?;
                file.set_len(0)?;
                file.write_all(text.as_bytes())
            })())
        };
        match &mut self.resource {
            Resource::Directory(dir) => {
                let (parent, name) = safe_error(access::secure_parent(dir, path))?;
                write(&mut safe_error(access::open_file(
                    &parent, &name, true, true,
                ))?)
            }
            Resource::File(file) => write(file),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<String>, String> {
        if !self.read {
            return Err("list denied".into());
        }
        let host = self.checked_path(path, false)?;
        let Resource::Directory(dir) = &self.resource else {
            return Err("not a directory".into());
        };
        use cap_fs_ext::DirExt;
        let (parent, name) = safe_error(access::secure_parent(dir, path))?;
        let dir = safe_error(parent.open_dir_nofollow(name))?;
        let mut names = vec![];
        for entry in safe_error(dir.entries())? {
            let entry = safe_error(entry)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if self.access.authorize(&host.join(&name), false).is_ok()
                && !safe_error(entry.file_type())?.is_symlink()
            {
                names.push(name);
            }
        }
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
