//! Invocation grants for standard WASI filesystem users (including CPython).
//! Directory preopens use Wasmtime confinement. A single-file preopen is a
//! synthetic mount point: only opening/statting `.` is allowed, and yields an
//! already-open file. Its parent directory is never handed to the guest.

use crate::{access::Access, runtime::State};
use anyhow::{Context, Result, ensure};
use std::path::Path;
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

pub fn configure(wasi: &mut WasiCtxBuilder, access: &Access) -> Result<Vec<(File, String)>> {
    let mut files = vec![];
    for root in &access.roots {
        let perms = if root.write {
            FsPerms::ReadWrite
        } else {
            FsPerms::ReadOnly
        };
        let mode = if root.write {
            OpenMode::READ | OpenMode::WRITE
        } else {
            OpenMode::READ
        };
        if root.directory {
            wasi.preopened_dir(&root.path, &root.guest, perms)?;
        } else {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(root.write)
                .open(&root.path)?;
            crate::access::check_file(&file)?;
            files.push((File::new(file, perms, mode, false), root.guest.clone()));
        }
    }
    Ok(files)
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
        if let Ok(types::Descriptor::File(file)) = self.table.get(fd) {
            crate::access::check_file(&file.file).map_err(|_| ErrorCode::NotPermitted)?;
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
        self.check_fd(&fd, true)?;
        self.native(&fd)?;
        HostDescriptor::set_size(&mut self.fs(), fd, size).await
    }

    async fn set_times(
        &mut self,
        fd: Resource<types::Descriptor>,
        atim: types::NewTimestamp,
        mtim: types::NewTimestamp,
    ) -> FsResult<()> {
        self.check_fd(&fd, true)?;
        self.native(&fd)?;
        HostDescriptor::set_times(&mut self.fs(), fd, atim, mtim).await
    }

    async fn read(
        &mut self,
        fd: Resource<types::Descriptor>,
        len: types::Filesize,
        offset: types::Filesize,
    ) -> FsResult<(Vec<u8>, bool)> {
        self.check_fd(&fd, false)?;
        self.native(&fd)?;
        HostDescriptor::read(&mut self.fs(), fd, len, offset).await
    }

    async fn write(
        &mut self,
        fd: Resource<types::Descriptor>,
        buf: Vec<u8>,
        offset: types::Filesize,
    ) -> FsResult<types::Filesize> {
        self.check_fd(&fd, true)?;
        self.native(&fd)?;
        HostDescriptor::write(&mut self.fs(), fd, buf, offset).await
    }

    async fn read_directory(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<Resource<types::DirectoryEntryStream>> {
        self.native(&fd)?;
        self.check_fd(&fd, false)?;
        let path = self
            .descriptor_paths
            .get(&fd.rep())
            .cloned()
            .ok_or(ErrorCode::NotPermitted)?;
        let stream = HostDescriptor::read_directory(&mut self.fs(), fd).await?;
        self.stream_paths.insert(stream.rep(), path);
        Ok(stream)
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
        self.check_at(&fd, &path, true)?;
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
        self.check_fd(&fd, false)?;
        self.native(&fd)?;
        HostDescriptor::stat(&mut self.fs(), fd).await
    }

    async fn stat_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::DescriptorStat> {
        self.check_at(&fd, &path, false)?;
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self
            .stat_at_resolved(fd, types::PathFlags::empty(), path)
            .await;
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
        self.check_at(&fd, &path, true)?;
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self
            .set_times_at_resolved(fd, types::PathFlags::empty(), path, atim, mtim)
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
        Err(ErrorCode::NotPermitted.into())
    }

    async fn open_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        _path_flags: types::PathFlags,
        path: String,
        oflags: types::OpenFlags,
        flags: types::DescriptorFlags,
    ) -> FsResult<Resource<types::Descriptor>> {
        let write = flags
            .intersects(types::DescriptorFlags::WRITE | types::DescriptorFlags::MUTATE_DIRECTORY)
            || oflags.intersects(types::OpenFlags::CREATE | types::OpenFlags::TRUNCATE);
        if flags.contains(types::DescriptorFlags::READ) || !write {
            self.check_at(&fd, &path, false)?;
        }
        if write {
            self.check_at(&fd, &path, true)?;
        }
        let plan = self.resolve(&fd, &path)?;
        let (fd, path, temp, guest) = self.materialize(fd, path, plan)?;
        let result = self
            .open_at_resolved(
                fd,
                types::PathFlags::empty(),
                path,
                oflags & !types::OpenFlags::TRUNCATE,
                flags,
            )
            .await;
        if let Some(rep) = temp {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        if let Ok(opened) = &result {
            if let Some(guest) = guest {
                self.descriptor_paths.insert(opened.rep(), guest);
            }
            if let Ok(types::Descriptor::File(file)) = self.table.get(opened) {
                crate::access::check_file(&file.file).map_err(|_| ErrorCode::NotPermitted)?;
            }
            if oflags.contains(types::OpenFlags::TRUNCATE) {
                HostDescriptor::set_size(self, Resource::new_borrow(opened.rep()), 0).await?;
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
        self.check_at(&fd, &path, false)?;
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
        self.check_at(&fd, &path, true)?;
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
        Err(ErrorCode::NotPermitted.into())
    }

    async fn symlink_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        src_path: String,
        dest_path: String,
    ) -> FsResult<()> {
        Err(ErrorCode::NotPermitted.into())
    }

    async fn unlink_file_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.check_at(&fd, &path, true)?;
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
        self.check_fd(&fd, false)?;
        self.native(&fd)?;
        HostDescriptor::read_via_stream(&mut self.fs(), fd, offset)
    }

    fn write_via_stream(
        &mut self,
        fd: Resource<types::Descriptor>,
        offset: types::Filesize,
    ) -> FsResult<Resource<DynOutputStream>> {
        self.check_fd(&fd, true)?;
        self.native(&fd)?;
        HostDescriptor::write_via_stream(&mut self.fs(), fd, offset)
    }

    fn append_via_stream(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<Resource<DynOutputStream>> {
        self.check_fd(&fd, true)?;
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
        self.check_fd(&fd, false)?;
        self.native(&fd)?;
        HostDescriptor::metadata_hash(&mut self.fs(), fd).await
    }

    async fn metadata_hash_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::MetadataHashValue> {
        self.check_at(&fd, &path, false)?;
        let plan0 = self.resolve(&fd, &path)?;
        let (fd, path, temp0, guest0) = self.materialize(fd, path, plan0)?;
        let result = self
            .metadata_hash_at_resolved(fd, types::PathFlags::empty(), path)
            .await;
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
        let base = self
            .stream_paths
            .get(&stream.rep())
            .cloned()
            .ok_or(ErrorCode::NotPermitted)?;
        loop {
            let entry = HostDirectoryEntryStream::read_directory_entry(
                &mut self.fs(),
                Resource::new_borrow(stream.rep()),
            )
            .await?;
            let Some(entry) = entry else {
                return Ok(None);
            };
            let guest = format!("{}/{}", base.trim_end_matches('/'), entry.name);
            if entry.type_ != types::DescriptorType::SymbolicLink
                && self.guest_authorize(&guest, false).is_ok()
            {
                return Ok(Some(entry));
            }
        }
    }

    fn drop(&mut self, stream: Resource<types::DirectoryEntryStream>) -> wasmtime::Result<()> {
        self.stream_paths.remove(&stream.rep());
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
        HostDescriptor::set_times_at(
            &mut self.fs(),
            fd,
            types::PathFlags::empty(),
            path,
            atim,
            mtim,
        )
        .await
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
    fn guest_authorize(&self, guest: &str, write: bool) -> FsResult<()> {
        let host = self
            .access
            .host_for_guest(guest)
            .ok_or(ErrorCode::NotPermitted)?;
        self.access
            .authorize(&host, write)
            .map_err(|_| ErrorCode::NotPermitted.into())
    }

    fn check_fd(&self, fd: &Resource<types::Descriptor>, write: bool) -> FsResult<()> {
        let guest = self
            .descriptor_paths
            .get(&fd.rep())
            .ok_or(ErrorCode::NotPermitted)?;
        self.guest_authorize(guest, write)
    }

    fn check_at(&self, fd: &Resource<types::Descriptor>, path: &str, write: bool) -> FsResult<()> {
        let base = self
            .descriptor_paths
            .get(&fd.rep())
            .ok_or(ErrorCode::NotPermitted)?;
        if path.starts_with('/') || path.contains('\\') {
            return Err(ErrorCode::NotPermitted.into());
        }
        let absolute = normalize(&format!("{base}/{path}")).map_err(|_| ErrorCode::NotPermitted)?;
        self.guest_authorize(&absolute, write)
    }

    fn resolve(&self, fd: &Resource<types::Descriptor>, path: &str) -> FsResult<Option<Resolved>> {
        let base = self
            .descriptor_paths
            .get(&fd.rep())
            .ok_or(ErrorCode::NotPermitted)?;
        if path.starts_with('/') || path.contains('\\') {
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
            GrantRoot::Native(types::Descriptor::Dir(d)) => {
                let root = cap_std::fs::Dir::from_std_file(d.dir.try_clone()?);
                let (parent, name) = crate::access::secure_parent(&root, &plan.relative)
                    .map_err(|_| ErrorCode::NotPermitted)?;
                let descriptor = types::Descriptor::Dir(wasmtime_wasi::filesystem::Dir::new(
                    parent.into_std_file(),
                    d.perms,
                    d.open_mode,
                    false,
                ));
                let fd = self.table.push(descriptor)?;
                let rep = fd.rep();
                return Ok((fd, name, Some(rep), Some(plan.absolute)));
            }
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
