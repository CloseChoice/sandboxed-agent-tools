//! Invocation grants for standard WASI filesystem users (including CPython).
//! Directory preopens use Wasmtime confinement. A single-file preopen is a
//! synthetic mount point: only opening/statting `.` is allowed, and yields an
//! already-open file. Its parent directory is never handed to the guest.
use crate::{metadata::Policy, runtime::State};
use anyhow::{Context, Result, ensure};
use clap::ArgMatches;
use std::path::{Component, PathBuf};
use wasmtime::component::{HasData, Linker, Resource};
use wasmtime_wasi::p2::bindings::filesystem::{
    preopens,
    types::{self, ErrorCode, HostDescriptor, HostDirectoryEntryStream},
};
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, FsError, FsResult};
use wasmtime_wasi::{
    WasiCtxBuilder,
    filesystem::{File, FsPerms, OpenMode, WasiFilesystemCtxView},
};

pub fn configure(
    wasi: &mut WasiCtxBuilder,
    args: &ArgMatches,
    policy: &Policy,
) -> Result<(Vec<(File, String)>, Option<String>)> {
    let mut paths = std::collections::BTreeMap::<(PathBuf, PathBuf), bool>::new();
    for (flag, write) in [("read", false), ("read-write", true)] {
        for path in args.get_many::<String>(flag).into_iter().flatten() {
            let (host, guest) = path.split_once("::").unwrap_or((path, path));
            ensure!(
                !host.is_empty() && !guest.is_empty() && !guest.contains("::"),
                "grant must be PATH or HOST::GUEST with nonempty paths"
            );
            paths
                .entry((PathBuf::from(host), PathBuf::from(guest)))
                .and_modify(|w| *w |= write)
                .or_insert(write);
        }
    }
    if paths.is_empty() {
        return Ok((Vec::new(), None));
    }
    // Existing policy lists refer to named WIT bindings. An unnamed grant must
    // not be an alternate route around those ceilings (including empty lists).
    ensure!(
        policy.files.is_none() && policy.folders.is_none(),
        "embedded filesystem policy requires named bindings; --read/--read-write cannot override it"
    );
    let cwd = std::env::current_dir()?;
    // Include only the ancestry explicitly requested by relative grants. Keep
    // its real basenames so ../<cwd-name>/file and ./file identify the same file.
    let depth = paths
        .keys()
        .map(|(_, guest)| guest)
        .filter(|p| p.is_relative())
        .map(|p| {
            let (mut level, mut parents) = (0_i32, 0_i32);
            for c in p.components() {
                match c {
                    Component::ParentDir => level -= 1,
                    Component::Normal(_) => level += 1,
                    _ => {}
                }
                parents = parents.max(-level);
            }
            parents as usize
        })
        .max()
        .unwrap_or(0);
    let mut ancestry = Vec::new();
    let mut ancestor = cwd.as_path();
    for _ in 0..depth {
        ancestry.push(
            ancestor
                .file_name()
                .context("grant escapes host root")?
                .to_str()
                .context("grant paths must be UTF-8")?,
        );
        ancestor = ancestor.parent().context("grant escapes host root")?;
    }
    ancestry.reverse();
    let guest_cwd = if ancestry.is_empty() {
        "/work".into()
    } else {
        format!("/work/{}", ancestry.join("/"))
    };
    wasi.env("PWD", &guest_cwd);
    let mut files = Vec::new();
    let mut cwd_covered = false;
    let mut destinations = std::collections::BTreeMap::new();
    for ((path, guest_path), write) in paths {
        ensure!(
            !guest_path
                .components()
                .any(|c| matches!(c, Component::Prefix(_))),
            "absolute drive paths are not supported; use a relative grant"
        );
        let text = guest_path.to_str().context("guest paths must be UTF-8")?;
        let text = if cfg!(windows) {
            text.replace('\\', "/")
        } else {
            text.to_owned()
        };
        let guest = if guest_path.is_absolute() {
            text
        } else {
            format!("{guest_cwd}/{text}")
        };
        let guest = normalize(&guest)?;
        let host = std::fs::canonicalize(&path).context("grant path must already exist")?;
        if let Some(previous) = destinations.insert(guest.clone(), host.clone()) {
            ensure!(
                previous == host,
                "grant paths collide in the guest filesystem"
            );
        }
        let perms = if write {
            FsPerms::ReadWrite
        } else {
            FsPerms::ReadOnly
        };
        let mode = if write {
            OpenMode::READ | OpenMode::WRITE
        } else {
            OpenMode::READ
        };
        let metadata = std::fs::metadata(&path).context("grant path must already exist")?;
        if metadata.is_dir() {
            cwd_covered |= guest_cwd == guest || guest_cwd.starts_with(&format!("{guest}/"));
            wasi.preopened_dir(&path, &guest, perms)?;
        } else {
            ensure!(
                metadata.is_file(),
                "only regular files and directories can be granted"
            );
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .open(&path)?;
            ensure!(file.metadata()?.is_file(), "grant must be a regular file");
            files.push((File::new(file, perms, mode, false), guest));
        }
    }
    Ok((files, (!cwd_covered).then_some(guest_cwd)))
}

fn normalize(path: &str) -> Result<String> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                ensure!(parts.pop().is_some(), "path escapes guest root");
            }
            part => parts.push(part),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

#[derive(Clone)]
pub(crate) enum Mount {
    File(File),
    WorkingDirectory,
}

fn directory_stat() -> types::DescriptorStat {
    types::DescriptorStat {
        type_: types::DescriptorType::Directory,
        link_count: 1,
        size: 0,
        data_access_timestamp: None,
        data_modification_timestamp: None,
        status_change_timestamp: None,
    }
}
struct GrantView;
impl HasData for GrantView {
    type Data<'a> = &'a mut State;
}

pub fn link(linker: &mut Linker<State>) -> Result<()> {
    linker.allow_shadowing(true);
    types::add_to_linker::<State, GrantView>(linker, |s| s)?;
    preopens::add_to_linker::<State, GrantView>(linker, |s| s)?;
    linker.allow_shadowing(false);
    Ok(())
}

impl State {
    fn fs(&mut self) -> WasiFilesystemCtxView<'_> {
        WasiFilesystemCtxView {
            ctx: self.wasi.filesystem(),
            table: &mut self.table,
        }
    }
    fn virtual_mount(&self, fd: &Resource<types::Descriptor>) -> bool {
        self.table
            .get(&Resource::<Mount>::new_borrow(fd.rep()))
            .is_ok()
    }
    fn cwd_mount(&self, fd: &Resource<types::Descriptor>) -> bool {
        matches!(
            self.table.get(&Resource::<Mount>::new_borrow(fd.rep())),
            Ok(Mount::WorkingDirectory)
        )
    }
    fn single(&self, fd: &Resource<types::Descriptor>) -> Option<File> {
        self.table
            .get(&Resource::<Mount>::new_borrow(fd.rep()))
            .ok()
            .and_then(|s| match s {
                Mount::File(f) => Some(f.clone()),
                Mount::WorkingDirectory => None,
            })
    }
    fn native(&self, fd: &Resource<types::Descriptor>) -> FsResult<()> {
        if self.virtual_mount(fd) {
            return Err(ErrorCode::NotPermitted.into());
        }
        Ok(())
    }
    fn single_path(path: &str) -> FsResult<()> {
        if path != "." && !path.is_empty() {
            return Err(ErrorCode::NotPermitted.into());
        }
        Ok(())
    }
}
impl preopens::Host for State {
    fn get_directories(&mut self) -> wasmtime::Result<Vec<(Resource<types::Descriptor>, String)>> {
        let mut dirs = preopens::Host::get_directories(&mut self.fs())?;
        for (file, path) in &self.single_files {
            let r = self.table.push(Mount::File(file.clone()))?;
            dirs.push((Resource::new_own(r.rep()), path.clone()));
        }
        if let Some(path) = &self.guest_cwd {
            let r = self.table.push(Mount::WorkingDirectory)?;
            dirs.push((Resource::new_own(r.rep()), path.clone()));
        }
        self.grant_roots.clear();
        for (fd, path) in &dirs {
            self.descriptor_paths.insert(fd.rep(), path.clone());
            let mount = if let Ok(m) = self.table.get(&Resource::<Mount>::new_borrow(fd.rep())) {
                GrantRoot::Virtual(m.clone())
            } else {
                GrantRoot::Native(self.table.get(fd)?.clone())
            };
            self.grant_roots.push((path.clone(), mount));
        }
        Ok(dirs)
    }
}
impl types::Host for State {
    fn convert_error_code(&mut self, err: FsError) -> wasmtime::Result<ErrorCode> {
        err.downcast()
    }
    fn filesystem_error_code(
        &mut self,
        err: Resource<wasmtime::Error>,
    ) -> wasmtime::Result<Option<ErrorCode>> {
        types::Host::filesystem_error_code(&mut self.fs(), err)
    }
}
impl HostDescriptor for State {
    async fn advise(
        &mut self,
        fd: Resource<types::Descriptor>,
        offset: types::Filesize,
        len: types::Filesize,
        advice: types::Advice,
    ) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::advise(&mut self.fs(), fd, offset, len, advice).await
    }
    async fn sync_data(&mut self, fd: Resource<types::Descriptor>) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::sync_data(&mut self.fs(), fd).await
    }
    async fn get_flags(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<types::DescriptorFlags> {
        if self.virtual_mount(&fd) {
            return Ok(types::DescriptorFlags::READ);
        }
        self.native(&fd)?;
        HostDescriptor::get_flags(&mut self.fs(), fd).await
    }
    async fn get_type(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<types::DescriptorType> {
        if self.virtual_mount(&fd) {
            return Ok(types::DescriptorType::Directory);
        }
        self.native(&fd)?;
        HostDescriptor::get_type(&mut self.fs(), fd).await
    }
    async fn set_size(
        &mut self,
        fd: Resource<types::Descriptor>,
        size: types::Filesize,
    ) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::set_size(&mut self.fs(), fd, size).await
    }
    async fn set_times(
        &mut self,
        fd: Resource<types::Descriptor>,
        atim: types::NewTimestamp,
        mtim: types::NewTimestamp,
    ) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::set_times(&mut self.fs(), fd, atim, mtim).await
    }
    async fn read(
        &mut self,
        fd: Resource<types::Descriptor>,
        len: types::Filesize,
        offset: types::Filesize,
    ) -> FsResult<(Vec<u8>, bool)> {
        self.native(&fd)?;
        HostDescriptor::read(&mut self.fs(), fd, len, offset).await
    }
    async fn write(
        &mut self,
        fd: Resource<types::Descriptor>,
        buf: Vec<u8>,
        offset: types::Filesize,
    ) -> FsResult<types::Filesize> {
        self.native(&fd)?;
        HostDescriptor::write(&mut self.fs(), fd, buf, offset).await
    }
    async fn read_directory(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<Resource<types::DirectoryEntryStream>> {
        self.native(&fd)?;
        HostDescriptor::read_directory(&mut self.fs(), fd).await
    }
    async fn sync(&mut self, fd: Resource<types::Descriptor>) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::sync(&mut self.fs(), fd).await
    }
    async fn create_directory_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self.create_directory_at_resolved(fd, path).await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        result
    }
    async fn stat(&mut self, fd: Resource<types::Descriptor>) -> FsResult<types::DescriptorStat> {
        if self.virtual_mount(&fd) {
            return Ok(directory_stat());
        }
        self.native(&fd)?;
        HostDescriptor::stat(&mut self.fs(), fd).await
    }
    async fn stat_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::DescriptorStat> {
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self.stat_at_resolved(fd, path_flags, path).await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        result
    }
    async fn set_times_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
        atim: types::NewTimestamp,
        mtim: types::NewTimestamp,
    ) -> FsResult<()> {
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self
            .set_times_at_resolved(fd, path_flags, path, atim, mtim)
            .await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        result
    }
    async fn link_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        old_path_flags: types::PathFlags,
        old_path: String,
        new_descriptor: Resource<types::Descriptor>,
        new_path: String,
    ) -> FsResult<()> {
        let plan0 = self.resolve(&fd, &old_path)?;
        let plan1 = self.resolve(&new_descriptor, &new_path)?;
        let (fd, old_path, temp0, guest0) = self.materialize(fd, old_path, plan0)?;
        let (new_descriptor, new_path, temp1, guest1) =
            self.materialize(new_descriptor, new_path, plan1)?;
        let result = self
            .link_at_resolved(fd, old_path_flags, old_path, new_descriptor, new_path)
            .await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        if let Some(rep) = temp1 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        let _ = guest1;
        result
    }
    async fn open_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
        oflags: types::OpenFlags,
        flags: types::DescriptorFlags,
    ) -> FsResult<Resource<types::Descriptor>> {
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self
            .open_at_resolved(fd, path_flags, path, oflags, flags)
            .await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        if let (Ok(fd), Some(path)) = (&result, guest0) {
            if self.virtual_mount(fd) || matches!(self.table.get(fd), Ok(types::Descriptor::Dir(_)))
            {
                self.descriptor_paths.insert(fd.rep(), path);
            }
        }
        result
    }
    fn drop(&mut self, fd: Resource<types::Descriptor>) -> wasmtime::Result<()> {
        self.descriptor_paths.remove(&fd.rep());
        if self.virtual_mount(&fd) {
            self.table.delete(Resource::<Mount>::new_own(fd.rep()))?;
            return Ok(());
        }
        self.native(&fd)?;
        HostDescriptor::drop(&mut self.fs(), fd)
    }
    async fn readlink_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<String> {
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self.readlink_at_resolved(fd, path).await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        result
    }
    async fn remove_directory_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self.remove_directory_at_resolved(fd, path).await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        result
    }
    async fn rename_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        old_path: String,
        new_fd: Resource<types::Descriptor>,
        new_path: String,
    ) -> FsResult<()> {
        let plan0 = self.resolve(&fd, &old_path)?;
        let plan1 = self.resolve(&new_fd, &new_path)?;
        let (fd, old_path, temp0, guest0) = self.materialize(fd, old_path, plan0)?;
        let (new_fd, new_path, temp1, guest1) = self.materialize(new_fd, new_path, plan1)?;
        let result = self
            .rename_at_resolved(fd, old_path, new_fd, new_path)
            .await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        if let Some(rep) = temp1 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        let _ = guest1;
        result
    }
    async fn symlink_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        src_path: String,
        dest_path: String,
    ) -> FsResult<()> {
        let plan0 = self.resolve(&fd, &dest_path)?;
        let (fd, dest_path, temp0, guest0) = self.materialize(fd, dest_path, plan0)?;
        let result = self.symlink_at_resolved(fd, src_path, dest_path).await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        result
    }
    async fn unlink_file_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self.unlink_file_at_resolved(fd, path).await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        result
    }
    fn read_via_stream(
        &mut self,
        fd: Resource<types::Descriptor>,
        offset: types::Filesize,
    ) -> FsResult<Resource<DynInputStream>> {
        self.native(&fd)?;
        HostDescriptor::read_via_stream(&mut self.fs(), fd, offset)
    }
    fn write_via_stream(
        &mut self,
        fd: Resource<types::Descriptor>,
        offset: types::Filesize,
    ) -> FsResult<Resource<DynOutputStream>> {
        self.native(&fd)?;
        HostDescriptor::write_via_stream(&mut self.fs(), fd, offset)
    }
    fn append_via_stream(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<Resource<DynOutputStream>> {
        self.native(&fd)?;
        HostDescriptor::append_via_stream(&mut self.fs(), fd)
    }
    async fn is_same_object(
        &mut self,
        a: Resource<types::Descriptor>,
        b: Resource<types::Descriptor>,
    ) -> wasmtime::Result<bool> {
        if self.virtual_mount(&a) || self.virtual_mount(&b) {
            return Ok(a.rep() == b.rep());
        }
        self.native(&a)?;
        self.native(&b)?;
        HostDescriptor::is_same_object(&mut self.fs(), a, b).await
    }
    async fn metadata_hash(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<types::MetadataHashValue> {
        if self.virtual_mount(&fd) {
            return Ok(types::MetadataHashValue {
                lower: fd.rep() as u64,
                upper: 0,
            });
        }
        self.native(&fd)?;
        HostDescriptor::metadata_hash(&mut self.fs(), fd).await
    }
    async fn metadata_hash_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::MetadataHashValue> {
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self.metadata_hash_at_resolved(fd, path_flags, path).await;
        if let Some(rep) = temp0 {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest0;
        result
    }
}
impl HostDirectoryEntryStream for State {
    async fn read_directory_entry(
        &mut self,
        stream: Resource<types::DirectoryEntryStream>,
    ) -> FsResult<Option<types::DirectoryEntry>> {
        HostDirectoryEntryStream::read_directory_entry(&mut self.fs(), stream).await
    }
    fn drop(&mut self, stream: Resource<types::DirectoryEntryStream>) -> wasmtime::Result<()> {
        HostDirectoryEntryStream::drop(&mut self.fs(), stream)
    }
}

impl State {
    async fn open_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
        oflags: types::OpenFlags,
        flags: types::DescriptorFlags,
    ) -> FsResult<Resource<types::Descriptor>> {
        if self.cwd_mount(&fd) {
            Self::single_path(&path)?;
            if oflags.intersects(types::OpenFlags::CREATE | types::OpenFlags::TRUNCATE)
                || flags.intersects(
                    types::DescriptorFlags::WRITE | types::DescriptorFlags::MUTATE_DIRECTORY,
                )
            {
                return Err(ErrorCode::NotPermitted.into());
            }
            return Ok(Resource::new_own(
                self.table.push(Mount::WorkingDirectory)?.rep(),
            ));
        }
        if let Some(mut file) = self.single(&fd) {
            Self::single_path(&path)?;
            if oflags.contains(types::OpenFlags::DIRECTORY) {
                return Err(ErrorCode::NotDirectory.into());
            }
            if oflags.contains(types::OpenFlags::CREATE | types::OpenFlags::EXCLUSIVE) {
                return Err(ErrorCode::Exist.into());
            }
            let write = flags.contains(types::DescriptorFlags::WRITE);
            if (write || oflags.contains(types::OpenFlags::TRUNCATE))
                && file.perms == FsPerms::ReadOnly
            {
                return Err(ErrorCode::NotPermitted.into());
            }
            if oflags.contains(types::OpenFlags::TRUNCATE) {
                if !write {
                    return Err(ErrorCode::NotPermitted.into());
                }
                file.file.set_len(0)?;
            }
            if !write {
                file.perms = FsPerms::ReadOnly;
            }
            file.open_mode = if write {
                OpenMode::READ | OpenMode::WRITE
            } else {
                OpenMode::READ
            };
            return Ok(self.table.push(types::Descriptor::File(file))?);
        }
        self.native(&fd)?;
        HostDescriptor::open_at(&mut self.fs(), fd, path_flags, path, oflags, flags).await
    }
    async fn stat_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::DescriptorStat> {
        if self.cwd_mount(&fd) {
            Self::single_path(&path)?;
            return Ok(directory_stat());
        }
        if let Some(file) = self.single(&fd) {
            Self::single_path(&path)?;
            let native = self.table.push(types::Descriptor::File(file))?;
            let result =
                HostDescriptor::stat(&mut self.fs(), Resource::new_borrow(native.rep())).await;
            self.table.delete(native)?;
            return result;
        }
        self.native(&fd)?;
        HostDescriptor::stat_at(&mut self.fs(), fd, path_flags, path).await
    }
    async fn metadata_hash_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::MetadataHashValue> {
        if self.cwd_mount(&fd) {
            Self::single_path(&path)?;
            return Ok(types::MetadataHashValue {
                lower: fd.rep() as u64,
                upper: 0,
            });
        }
        if let Some(file) = self.single(&fd) {
            Self::single_path(&path)?;
            let native = self.table.push(types::Descriptor::File(file))?;
            let result =
                HostDescriptor::metadata_hash(&mut self.fs(), Resource::new_borrow(native.rep()))
                    .await;
            self.table.delete(native)?;
            return result;
        }
        self.native(&fd)?;
        HostDescriptor::metadata_hash_at(&mut self.fs(), fd, path_flags, path).await
    }
    async fn create_directory_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::create_directory_at(&mut self.fs(), fd, path).await
    }
    async fn set_times_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
        atim: types::NewTimestamp,
        mtim: types::NewTimestamp,
    ) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::set_times_at(&mut self.fs(), fd, path_flags, path, atim, mtim).await
    }
    async fn link_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        old_path_flags: types::PathFlags,
        old_path: String,
        new_descriptor: Resource<types::Descriptor>,
        new_path: String,
    ) -> FsResult<()> {
        self.native(&fd)?;
        self.native(&new_descriptor)?;
        HostDescriptor::link_at(
            &mut self.fs(),
            fd,
            old_path_flags,
            old_path,
            new_descriptor,
            new_path,
        )
        .await
    }
    async fn readlink_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<String> {
        self.native(&fd)?;
        HostDescriptor::readlink_at(&mut self.fs(), fd, path).await
    }
    async fn remove_directory_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::remove_directory_at(&mut self.fs(), fd, path).await
    }
    async fn rename_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        old_path: String,
        new_fd: Resource<types::Descriptor>,
        new_path: String,
    ) -> FsResult<()> {
        self.native(&fd)?;
        self.native(&new_fd)?;
        HostDescriptor::rename_at(&mut self.fs(), fd, old_path, new_fd, new_path).await
    }
    async fn symlink_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        src_path: String,
        dest_path: String,
    ) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::symlink_at(&mut self.fs(), fd, src_path, dest_path).await
    }
    async fn unlink_file_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.native(&fd)?;
        HostDescriptor::unlink_file_at(&mut self.fs(), fd, path).await
    }
}

// A snapshot of the granted roots, independent of guest descriptor lifetimes.
// Routing `..` can switch roots, but can never manufacture a new host grant.
#[derive(Clone)]
pub(crate) enum GrantRoot {
    Native(types::Descriptor),
    Virtual(Mount),
}
struct Resolved {
    root: GrantRoot,
    relative: String,
    absolute: String,
}
impl State {
    fn resolve(&self, fd: &Resource<types::Descriptor>, path: &str) -> FsResult<Option<Resolved>> {
        let Some(base) = self.descriptor_paths.get(&fd.rep()) else {
            return Ok(None);
        };
        if path.starts_with('/') {
            return Err(ErrorCode::NotPermitted.into());
        }
        let absolute = normalize(&format!("{base}/{path}")).map_err(|_| ErrorCode::NotPermitted)?;
        let (root_path, root) = self
            .grant_roots
            .iter()
            .filter(|(p, _)| {
                absolute == *p || absolute.starts_with(&format!("{}/", p.trim_end_matches('/')))
            })
            .max_by_key(|(p, r)| (r.writable(), p.len()))
            .ok_or(ErrorCode::NotPermitted)?;
        let relative = absolute
            .strip_prefix(root_path)
            .unwrap()
            .trim_start_matches('/');
        Ok(Some(Resolved {
            root: root.clone(),
            relative: if relative.is_empty() {
                ".".into()
            } else {
                relative.into()
            },
            absolute,
        }))
    }
    fn materialize(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
        plan: Option<Resolved>,
    ) -> FsResult<(
        Resource<types::Descriptor>,
        String,
        Option<u32>,
        Option<String>,
    )> {
        let Some(plan) = plan else {
            return Ok((fd, path, None, None));
        };
        let fd = match plan.root {
            GrantRoot::Native(d) => self.table.push(d)?,
            GrantRoot::Virtual(m) => Resource::new_own(self.table.push(m)?.rep()),
        };
        let rep = fd.rep();
        Ok((fd, plan.relative, Some(rep), Some(plan.absolute)))
    }
}
impl GrantRoot {
    fn writable(&self) -> bool {
        match self {
            Self::Native(types::Descriptor::Dir(d)) => d.perms == FsPerms::ReadWrite,
            Self::Native(types::Descriptor::File(f)) | Self::Virtual(Mount::File(f)) => {
                f.perms == FsPerms::ReadWrite
            }
            Self::Virtual(Mount::WorkingDirectory) => false,
        }
    }
}
