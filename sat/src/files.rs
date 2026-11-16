//! MVP convention: named imported interfaces expose text read/write and directory list.

use crate::{
    access::{self, Access},
    runtime::RuntimeState,
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
/// Operations requested by a named component import; these grant no authority.
pub struct FilesystemImport {
    pub name: String,
    pub directory: bool,
    pub read: bool,
    pub write: bool,
    pub binary_write: bool,
}

pub fn discover_filesystem_imports(
    engine: &Engine,
    component: &Component,
) -> Result<Vec<FilesystemImport>> {
    let mut filesystem_imports = Vec::new();
    for (name, item) in component.component_type().imports(engine) {
        if name == crate::browser::INTERFACE {
            continue;
        }
        if name.starts_with("wasi:") {
            continue;
        }
        let ComponentItem::ComponentInstance(interface) = item.ty else {
            bail!("unsupported import {name}");
        };
        let functions: Vec<_> = interface.exports(engine).collect();
        let directory = functions.iter().any(|(n, _)| *n == "list");
        let filesystem_import = FilesystemImport {
            name: name.into(),
            directory,
            read: functions.iter().any(|(n, _)| *n == "read"),
            write: functions.iter().any(|(n, _)| *n == "write"),
            binary_write: functions.iter().any(|(n, item)| *n == "write" && matches!(&item.ty, ComponentItem::ComponentFunc(f) if matches!(f.params().last(), Some((_, Type::List(l))) if matches!(l.ty(), Type::U8)))),
        };
        ensure!(
            filesystem_import.read || filesystem_import.write,
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
                    && f.params().enumerate().all(|(i, (_, t))| {
                        matches!(t, Type::String)
                            || (operation == "write"
                                && i == count - 1
                                && matches!(t, Type::List(l) if matches!(l.ty(), Type::U8)))
                    }),
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
        filesystem_imports.push(filesystem_import);
    }
    Ok(filesystem_imports)
}

/// An opened resource with permissions limited by host grants.
pub struct FilesystemBinding {
    resource: BoundFilesystemResource,
    read: bool,
    write: bool,
    path: PathBuf,
    access: Access,
}

enum BoundFilesystemResource {
    Directory(Dir),
    File(File),
}

pub fn bind_filesystem_import(
    filesystem_import: &FilesystemImport,
    configured_path: &str,
    access: &Access,
    home: &Path,
    cwd: &Path,
) -> Result<FilesystemBinding> {
    let path = access::expand_path(configured_path, home, cwd)?;
    let path = std::fs::canonicalize(path)?;
    // Binding a resource does not itself widen its granted authority.
    let read = filesystem_import.read && access.authorize(&path, false).is_ok();
    let write = filesystem_import.write && access.authorize(&path, true).is_ok();
    ensure!(
        read || write,
        "no matching grant for binding {}",
        filesystem_import.name
    );
    let grant = access
        .grants
        .iter()
        .filter(|r| path == r.path || r.directory && path.starts_with(&r.path))
        .max_by_key(|r| r.path.components().count())
        .ok_or_else(|| anyhow::anyhow!("binding not granted"))?;
    let resource = if grant.directory {
        let dir = Dir::open_ambient_dir(&grant.path, cap_std::ambient_authority())?;
        let relative_path = path
            .strip_prefix(&grant.path)?
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non UTF-8 binding"))?;
        let (parent, name) = access::open_parent_directory_without_symlinks(&dir, relative_path)?;
        if filesystem_import.directory {
            use cap_fs_ext::DirExt;
            BoundFilesystemResource::Directory(parent.open_dir_nofollow(name)?)
        } else {
            BoundFilesystemResource::File(access::open_file(&parent, &name, write, false)?)
        }
    } else {
        ensure!(!filesystem_import.directory, "binding requires directory");
        // Explicit file grants pin the selected inode without exposing its parent.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .open(&path)?;
        access::check_file(&file)?;
        BoundFilesystemResource::File(file)
    };
    Ok(FilesystemBinding {
        resource,
        read,
        write,
        path,
        access: access.clone(),
    })
}

fn hide_host_path_in_error<T>(r: std::io::Result<T>) -> Result<T, String> {
    // Do not disclose host paths in filesystem error messages.
    r.map_err(|e| format!("filesystem operation failed: {:?}", e.kind()))
}

impl FilesystemBinding {
    fn authorize_relative_path(&self, path: &str, write: bool) -> Result<PathBuf, String> {
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
        self.authorize_relative_path(path, false)?;
        let mut read = |file: &mut File| {
            hide_host_path_in_error((|| {
                access::check_file(file)?;
                file.seek(SeekFrom::Start(0))?;
                let mut text = String::new();
                file.read_to_string(&mut text)?;
                Ok(text)
            })())
        };
        match &mut self.resource {
            BoundFilesystemResource::Directory(dir) => {
                let (parent, name) = hide_host_path_in_error(
                    access::open_parent_directory_without_symlinks(dir, path),
                )?;
                read(&mut hide_host_path_in_error(access::open_file(
                    &parent, &name, false, false,
                ))?)
            }
            BoundFilesystemResource::File(file) => read(file),
        }
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        if !self.write {
            return Err("write denied".into());
        }
        self.authorize_relative_path(path, true)?;
        let mut write = |file: &mut File| {
            hide_host_path_in_error((|| {
                access::check_file(file)?;
                file.seek(SeekFrom::Start(0))?;
                file.set_len(0)?;
                file.write_all(bytes)
            })())
        };
        match &mut self.resource {
            BoundFilesystemResource::Directory(dir) => {
                let (parent, name) = hide_host_path_in_error(
                    access::open_parent_directory_without_symlinks(dir, path),
                )?;
                write(&mut hide_host_path_in_error(access::open_file(
                    &parent, &name, true, true,
                ))?)
            }
            BoundFilesystemResource::File(file) => write(file),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<String>, String> {
        if !self.read {
            return Err("list denied".into());
        }
        let host = self.authorize_relative_path(path, false)?;
        let BoundFilesystemResource::Directory(dir) = &self.resource else {
            return Err("not a directory".into());
        };
        use cap_fs_ext::DirExt;
        let (parent, name) =
            hide_host_path_in_error(access::open_parent_directory_without_symlinks(dir, path))?;
        let dir = hide_host_path_in_error(parent.open_dir_nofollow(name))?;
        let mut names = vec![];
        for entry in hide_host_path_in_error(dir.entries())? {
            let entry = hide_host_path_in_error(entry)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if self.access.authorize(&host.join(&name), false).is_ok()
                && !hide_host_path_in_error(entry.file_type())?.is_symlink()
            {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }
}

pub fn link_filesystem_imports(
    linker: &mut Linker<RuntimeState>,
    filesystem_imports: &[FilesystemImport],
) -> Result<()> {
    for filesystem_import in filesystem_imports {
        let mut interface = linker.instance(&filesystem_import.name)?;
        if filesystem_import.read {
            let name = filesystem_import.name.clone();
            if filesystem_import.directory {
                interface.func_wrap(
                    "read",
                    move |mut s: StoreContextMut<'_, RuntimeState>, (path,): (String,)| {
                        Ok((s.data_mut().files.get_mut(&name).unwrap().read(&path),))
                    },
                )?;
            } else {
                interface.func_wrap(
                    "read",
                    move |mut s: StoreContextMut<'_, RuntimeState>, (): ()| {
                        Ok((s.data_mut().files.get_mut(&name).unwrap().read(""),))
                    },
                )?;
            }
        }
        if filesystem_import.write {
            let name = filesystem_import.name.clone();
            if filesystem_import.binary_write {
                if filesystem_import.directory {
                    interface.func_wrap(
                        "write",
                        move |mut s: StoreContextMut<'_, RuntimeState>,
                              (path, bytes): (String, Vec<u8>)| {
                            Ok((s
                                .data_mut()
                                .files
                                .get_mut(&name)
                                .unwrap()
                                .write(&path, &bytes),))
                        },
                    )?;
                } else {
                    interface.func_wrap(
                        "write",
                        move |mut s: StoreContextMut<'_, RuntimeState>, (bytes,): (Vec<u8>,)| {
                            Ok((s.data_mut().files.get_mut(&name).unwrap().write("", &bytes),))
                        },
                    )?;
                }
            } else if filesystem_import.directory {
                interface.func_wrap(
                    "write",
                    move |mut s: StoreContextMut<'_, RuntimeState>,
                          (path, text): (String, String)| {
                        Ok((s
                            .data_mut()
                            .files
                            .get_mut(&name)
                            .unwrap()
                            .write(&path, text.as_bytes()),))
                    },
                )?;
            } else {
                interface.func_wrap(
                    "write",
                    move |mut s: StoreContextMut<'_, RuntimeState>, (text,): (String,)| {
                        Ok((s
                            .data_mut()
                            .files
                            .get_mut(&name)
                            .unwrap()
                            .write("", text.as_bytes()),))
                    },
                )?;
            }
        }
        if filesystem_import.directory {
            let name = filesystem_import.name.clone();
            interface.func_wrap(
                "list",
                move |s: StoreContextMut<'_, RuntimeState>, (path,): (String,)| {
                    Ok((s.data().files[&name].list(&path),))
                },
            )?;
        }
    }
    Ok(())
}

pub type FilesystemBindings = BTreeMap<String, FilesystemBinding>;
