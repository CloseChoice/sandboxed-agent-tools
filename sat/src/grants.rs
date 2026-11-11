//! Invocation grants for standard WASI filesystem users (including CPython).
//! Directory preopens use Wasmtime confinement. A single-file preopen is a
//! synthetic mount point: only opening/statting `.` is allowed, and yields an
//! already-open file. Its parent directory is never handed to the guest.

use crate::{access::Access, runtime::RuntimeState};
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

pub fn configure_preopens(
    wasi: &mut WasiCtxBuilder,
    access: &Access,
) -> Result<Vec<(File, String)>> {
    let mut files = vec![];
    for grant in &access.grants {
        let perms = if grant.write {
            FsPerms::ReadWrite
        } else {
            FsPerms::ReadOnly
        };
        let mode = if grant.write {
            OpenMode::READ | OpenMode::WRITE
        } else {
            OpenMode::READ
        };
        if grant.directory {
            wasi.preopened_dir(&grant.path, &grant.guest, perms)?;
        } else {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(grant.write)
                .open(&grant.path)?;
            crate::access::check_file(&file)?;
            files.push((File::new(file, perms, mode, false), grant.guest.clone()));
        }
    }
    Ok(files)
}

fn normalize_guest_path(path: &str) -> Result<String> {
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

struct FilesystemHostView;

impl HasData for FilesystemHostView {
    type Data<'a> = &'a mut RuntimeState;
}

pub fn link_filesystem_permissions(linker: &mut Linker<RuntimeState>) -> Result<()> {
    linker.allow_shadowing(true);
    types::add_to_linker::<RuntimeState, FilesystemHostView>(linker, |s| s)?;
    preopens::add_to_linker::<RuntimeState, FilesystemHostView>(linker, |s| s)?;
    linker.allow_shadowing(false);
    Ok(())
}

impl RuntimeState {
    fn wasi_filesystem_view(&mut self) -> WasiFilesystemCtxView<'_> {
        WasiFilesystemCtxView {
            ctx: self.wasi.filesystem(),
            table: &mut self.table,
        }
    }

    fn is_virtual_mount(&self, fd: &Resource<types::Descriptor>) -> bool {
        self.table
            .get(&Resource::<Mount>::new_borrow(fd.rep()))
            .is_ok()
    }

    fn is_working_directory_mount(&self, fd: &Resource<types::Descriptor>) -> bool {
        matches!(
            self.table.get(&Resource::<Mount>::new_borrow(fd.rep())),
            Ok(Mount::WorkingDirectory)
        )
    }

    fn mounted_file(&self, fd: &Resource<types::Descriptor>) -> Option<File> {
        self.table
            .get(&Resource::<Mount>::new_borrow(fd.rep()))
            .ok()
            .and_then(|s| match s {
                Mount::File(f) => Some(f.clone()),
                Mount::WorkingDirectory => None,
            })
    }

    fn require_native_descriptor(&self, fd: &Resource<types::Descriptor>) -> FsResult<()> {
        if self.is_virtual_mount(fd) {
            return Err(ErrorCode::NotPermitted.into());
        }
        if let Ok(types::Descriptor::File(file)) = self.table.get(fd) {
            crate::access::check_file(&file.file).map_err(|_| ErrorCode::NotPermitted)?;
        }
        Ok(())
    }

    fn require_mount_self_path(path: &str) -> FsResult<()> {
        if path != "." && !path.is_empty() {
            return Err(ErrorCode::NotPermitted.into());
        }
        Ok(())
    }
}

impl preopens::Host for RuntimeState {
    fn get_directories(&mut self) -> wasmtime::Result<Vec<(Resource<types::Descriptor>, String)>> {
        let mut dirs = preopens::Host::get_directories(&mut self.wasi_filesystem_view())?;
        for (file, path) in &self.single_file_preopens {
            let r = self.table.push(Mount::File(file.clone()))?;
            dirs.push((Resource::new_own(r.rep()), path.clone()));
        }
        if let Some(path) = &self.guest_cwd {
            let r = self.table.push(Mount::WorkingDirectory)?;
            dirs.push((Resource::new_own(r.rep()), path.clone()));
        }
        self.granted_descriptors.clear();
        for (fd, path) in &dirs {
            self.descriptor_paths.insert(fd.rep(), path.clone());
            let mount = if let Ok(m) = self.table.get(&Resource::<Mount>::new_borrow(fd.rep())) {
                GrantedDescriptor::Virtual(m.clone())
            } else {
                GrantedDescriptor::Native(self.table.get(fd)?.clone())
            };
            self.granted_descriptors.push((path.clone(), mount));
        }
        Ok(dirs)
    }
}

impl types::Host for RuntimeState {
    fn convert_error_code(&mut self, err: FsError) -> wasmtime::Result<ErrorCode> {
        err.downcast()
    }

    fn filesystem_error_code(
        &mut self,
        err: Resource<wasmtime::Error>,
    ) -> wasmtime::Result<Option<ErrorCode>> {
        types::Host::filesystem_error_code(&mut self.wasi_filesystem_view(), err)
    }
}

impl HostDescriptor for RuntimeState {
    async fn advise(
        &mut self,
        fd: Resource<types::Descriptor>,
        offset: types::Filesize,
        len: types::Filesize,
        advice: types::Advice,
    ) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        HostDescriptor::advise(&mut self.wasi_filesystem_view(), fd, offset, len, advice).await
    }

    async fn sync_data(&mut self, fd: Resource<types::Descriptor>) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        HostDescriptor::sync_data(&mut self.wasi_filesystem_view(), fd).await
    }

    async fn get_flags(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<types::DescriptorFlags> {
        if self.is_virtual_mount(&fd) {
            return Ok(types::DescriptorFlags::READ);
        }
        self.require_native_descriptor(&fd)?;
        HostDescriptor::get_flags(&mut self.wasi_filesystem_view(), fd).await
    }

    async fn get_type(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<types::DescriptorType> {
        if self.is_virtual_mount(&fd) {
            return Ok(types::DescriptorType::Directory);
        }
        self.require_native_descriptor(&fd)?;
        HostDescriptor::get_type(&mut self.wasi_filesystem_view(), fd).await
    }

    async fn set_size(
        &mut self,
        fd: Resource<types::Descriptor>,
        size: types::Filesize,
    ) -> FsResult<()> {
        self.authorize_descriptor(&fd, true)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::set_size(&mut self.wasi_filesystem_view(), fd, size).await
    }

    async fn set_times(
        &mut self,
        fd: Resource<types::Descriptor>,
        atim: types::NewTimestamp,
        mtim: types::NewTimestamp,
    ) -> FsResult<()> {
        self.authorize_descriptor(&fd, true)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::set_times(&mut self.wasi_filesystem_view(), fd, atim, mtim).await
    }

    async fn read(
        &mut self,
        fd: Resource<types::Descriptor>,
        len: types::Filesize,
        offset: types::Filesize,
    ) -> FsResult<(Vec<u8>, bool)> {
        self.authorize_descriptor(&fd, false)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::read(&mut self.wasi_filesystem_view(), fd, len, offset).await
    }

    async fn write(
        &mut self,
        fd: Resource<types::Descriptor>,
        buf: Vec<u8>,
        offset: types::Filesize,
    ) -> FsResult<types::Filesize> {
        self.authorize_descriptor(&fd, true)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::write(&mut self.wasi_filesystem_view(), fd, buf, offset).await
    }

    async fn read_directory(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<Resource<types::DirectoryEntryStream>> {
        self.require_native_descriptor(&fd)?;
        self.authorize_descriptor(&fd, false)?;
        let path = self
            .descriptor_paths
            .get(&fd.rep())
            .cloned()
            .ok_or(ErrorCode::NotPermitted)?;
        let stream = HostDescriptor::read_directory(&mut self.wasi_filesystem_view(), fd).await?;
        self.directory_stream_paths.insert(stream.rep(), path);
        Ok(stream)
    }

    async fn sync(&mut self, fd: Resource<types::Descriptor>) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        HostDescriptor::sync(&mut self.wasi_filesystem_view(), fd).await
    }

    async fn create_directory_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.authorize_relative_guest_path(&fd, &path, true)?;
        let resolved_path = self.resolve_guest_path(&fd, &path)?;
        let (fd, path, temporary_descriptor_id, guest_path) =
            self.open_resolved_descriptor(fd, path, resolved_path)?;
        let result = self.create_directory_at_resolved(fd, path).await;
        if let Some(rep) = temporary_descriptor_id {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest_path;
        result
    }

    async fn stat(&mut self, fd: Resource<types::Descriptor>) -> FsResult<types::DescriptorStat> {
        if self.is_virtual_mount(&fd) {
            return Ok(directory_stat());
        }
        self.authorize_descriptor(&fd, false)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::stat(&mut self.wasi_filesystem_view(), fd).await
    }

    async fn stat_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::DescriptorStat> {
        self.authorize_relative_guest_path(&fd, &path, false)?;
        let resolved_path = self.resolve_guest_path(&fd, &path)?;
        let (fd, path, temporary_descriptor_id, guest_path) =
            self.open_resolved_descriptor(fd, path, resolved_path)?;
        let result = self
            .stat_at_resolved(fd, types::PathFlags::empty(), path)
            .await;
        if let Some(rep) = temporary_descriptor_id {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest_path;
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
        self.authorize_relative_guest_path(&fd, &path, true)?;
        let resolved_path = self.resolve_guest_path(&fd, &path)?;
        let (fd, path, temporary_descriptor_id, guest_path) =
            self.open_resolved_descriptor(fd, path, resolved_path)?;
        let result = self
            .set_times_at_resolved(fd, types::PathFlags::empty(), path, atim, mtim)
            .await;
        if let Some(rep) = temporary_descriptor_id {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest_path;
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
            self.authorize_relative_guest_path(&fd, &path, false)?;
        }
        if write {
            self.authorize_relative_guest_path(&fd, &path, true)?;
        }
        let resolved_path = self.resolve_guest_path(&fd, &path)?;
        let (fd, path, temporary_descriptor_id, guest_path) =
            self.open_resolved_descriptor(fd, path, resolved_path)?;
        let result = self
            .open_at_resolved(
                fd,
                types::PathFlags::empty(),
                path,
                oflags & !types::OpenFlags::TRUNCATE,
                flags,
            )
            .await;
        if let Some(rep) = temporary_descriptor_id {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        if let Ok(opened) = &result {
            if let Some(guest_path) = guest_path {
                self.descriptor_paths.insert(opened.rep(), guest_path);
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
        if self.is_virtual_mount(&fd) {
            self.table.delete(Resource::<Mount>::new_own(fd.rep()))?;
            return Ok(());
        }
        self.require_native_descriptor(&fd)?;
        HostDescriptor::drop(&mut self.wasi_filesystem_view(), fd)
    }

    async fn readlink_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<String> {
        self.authorize_relative_guest_path(&fd, &path, false)?;
        let resolved_path = self.resolve_guest_path(&fd, &path)?;
        let (fd, path, temporary_descriptor_id, guest_path) =
            self.open_resolved_descriptor(fd, path, resolved_path)?;
        let result = self.readlink_at_resolved(fd, path).await;
        if let Some(rep) = temporary_descriptor_id {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest_path;
        result
    }

    async fn remove_directory_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.authorize_relative_guest_path(&fd, &path, true)?;
        let resolved_path = self.resolve_guest_path(&fd, &path)?;
        let (fd, path, temporary_descriptor_id, guest_path) =
            self.open_resolved_descriptor(fd, path, resolved_path)?;
        let result = self.remove_directory_at_resolved(fd, path).await;
        if let Some(rep) = temporary_descriptor_id {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest_path;
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
        self.authorize_relative_guest_path(&fd, &path, true)?;
        let resolved_path = self.resolve_guest_path(&fd, &path)?;
        let (fd, path, temporary_descriptor_id, guest_path) =
            self.open_resolved_descriptor(fd, path, resolved_path)?;
        let result = self.unlink_file_at_resolved(fd, path).await;
        if let Some(rep) = temporary_descriptor_id {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest_path;
        result
    }

    fn read_via_stream(
        &mut self,
        fd: Resource<types::Descriptor>,
        offset: types::Filesize,
    ) -> FsResult<Resource<DynInputStream>> {
        self.authorize_descriptor(&fd, false)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::read_via_stream(&mut self.wasi_filesystem_view(), fd, offset)
    }

    fn write_via_stream(
        &mut self,
        fd: Resource<types::Descriptor>,
        offset: types::Filesize,
    ) -> FsResult<Resource<DynOutputStream>> {
        self.authorize_descriptor(&fd, true)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::write_via_stream(&mut self.wasi_filesystem_view(), fd, offset)
    }

    fn append_via_stream(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<Resource<DynOutputStream>> {
        self.authorize_descriptor(&fd, true)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::append_via_stream(&mut self.wasi_filesystem_view(), fd)
    }

    async fn is_same_object(
        &mut self,
        a: Resource<types::Descriptor>,
        b: Resource<types::Descriptor>,
    ) -> wasmtime::Result<bool> {
        if self.is_virtual_mount(&a) || self.is_virtual_mount(&b) {
            return Ok(a.rep() == b.rep());
        }
        self.require_native_descriptor(&a)?;
        self.require_native_descriptor(&b)?;
        HostDescriptor::is_same_object(&mut self.wasi_filesystem_view(), a, b).await
    }

    async fn metadata_hash(
        &mut self,
        fd: Resource<types::Descriptor>,
    ) -> FsResult<types::MetadataHashValue> {
        if self.is_virtual_mount(&fd) {
            return Ok(types::MetadataHashValue {
                lower: fd.rep() as u64,
                upper: 0,
            });
        }
        self.authorize_descriptor(&fd, false)?;
        self.require_native_descriptor(&fd)?;
        HostDescriptor::metadata_hash(&mut self.wasi_filesystem_view(), fd).await
    }

    async fn metadata_hash_at(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::MetadataHashValue> {
        self.authorize_relative_guest_path(&fd, &path, false)?;
        let resolved_path = self.resolve_guest_path(&fd, &path)?;
        let (fd, path, temporary_descriptor_id, guest_path) =
            self.open_resolved_descriptor(fd, path, resolved_path)?;
        let result = self
            .metadata_hash_at_resolved(fd, types::PathFlags::empty(), path)
            .await;
        if let Some(rep) = temporary_descriptor_id {
            HostDescriptor::drop(self, Resource::new_own(rep)).map_err(FsError::trap)?;
        }
        let _ = guest_path;
        result
    }
}

impl HostDirectoryEntryStream for RuntimeState {
    async fn read_directory_entry(
        &mut self,
        stream: Resource<types::DirectoryEntryStream>,
    ) -> FsResult<Option<types::DirectoryEntry>> {
        let base = self
            .directory_stream_paths
            .get(&stream.rep())
            .cloned()
            .ok_or(ErrorCode::NotPermitted)?;
        loop {
            let entry = HostDirectoryEntryStream::read_directory_entry(
                &mut self.wasi_filesystem_view(),
                Resource::new_borrow(stream.rep()),
            )
            .await?;
            let Some(entry) = entry else {
                return Ok(None);
            };
            let guest_path = format!("{}/{}", base.trim_end_matches('/'), entry.name);
            if entry.type_ != types::DescriptorType::SymbolicLink
                && self.authorize_guest_path(&guest_path, false).is_ok()
            {
                return Ok(Some(entry));
            }
        }
    }

    fn drop(&mut self, stream: Resource<types::DirectoryEntryStream>) -> wasmtime::Result<()> {
        self.directory_stream_paths.remove(&stream.rep());
        HostDirectoryEntryStream::drop(&mut self.wasi_filesystem_view(), stream)
    }
}

impl RuntimeState {
    async fn open_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
        oflags: types::OpenFlags,
        flags: types::DescriptorFlags,
    ) -> FsResult<Resource<types::Descriptor>> {
        if self.is_working_directory_mount(&fd) {
            Self::require_mount_self_path(&path)?;
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
        if let Some(mut file) = self.mounted_file(&fd) {
            Self::require_mount_self_path(&path)?;
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
        self.require_native_descriptor(&fd)?;
        HostDescriptor::open_at(
            &mut self.wasi_filesystem_view(),
            fd,
            path_flags,
            path,
            oflags,
            flags,
        )
        .await
    }

    async fn stat_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::DescriptorStat> {
        if self.is_working_directory_mount(&fd) {
            Self::require_mount_self_path(&path)?;
            return Ok(directory_stat());
        }
        if let Some(file) = self.mounted_file(&fd) {
            Self::require_mount_self_path(&path)?;
            let require_native_descriptor = self.table.push(types::Descriptor::File(file))?;
            let result = HostDescriptor::stat(
                &mut self.wasi_filesystem_view(),
                Resource::new_borrow(require_native_descriptor.rep()),
            )
            .await;
            self.table.delete(require_native_descriptor)?;
            return result;
        }
        self.require_native_descriptor(&fd)?;
        HostDescriptor::stat_at(&mut self.wasi_filesystem_view(), fd, path_flags, path).await
    }

    async fn metadata_hash_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
    ) -> FsResult<types::MetadataHashValue> {
        if self.is_working_directory_mount(&fd) {
            Self::require_mount_self_path(&path)?;
            return Ok(types::MetadataHashValue {
                lower: fd.rep() as u64,
                upper: 0,
            });
        }
        if let Some(file) = self.mounted_file(&fd) {
            Self::require_mount_self_path(&path)?;
            let require_native_descriptor = self.table.push(types::Descriptor::File(file))?;
            let result = HostDescriptor::metadata_hash(
                &mut self.wasi_filesystem_view(),
                Resource::new_borrow(require_native_descriptor.rep()),
            )
            .await;
            self.table.delete(require_native_descriptor)?;
            return result;
        }
        self.require_native_descriptor(&fd)?;
        HostDescriptor::metadata_hash_at(&mut self.wasi_filesystem_view(), fd, path_flags, path)
            .await
    }

    async fn create_directory_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        HostDescriptor::create_directory_at(&mut self.wasi_filesystem_view(), fd, path).await
    }

    async fn set_times_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path_flags: types::PathFlags,
        path: String,
        atim: types::NewTimestamp,
        mtim: types::NewTimestamp,
    ) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        HostDescriptor::set_times_at(
            &mut self.wasi_filesystem_view(),
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
        self.require_native_descriptor(&fd)?;
        self.require_native_descriptor(&new_descriptor)?;
        HostDescriptor::link_at(
            &mut self.wasi_filesystem_view(),
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
        self.require_native_descriptor(&fd)?;
        HostDescriptor::readlink_at(&mut self.wasi_filesystem_view(), fd, path).await
    }

    async fn remove_directory_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        HostDescriptor::remove_directory_at(&mut self.wasi_filesystem_view(), fd, path).await
    }

    async fn rename_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        old_path: String,
        new_fd: Resource<types::Descriptor>,
        new_path: String,
    ) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        self.require_native_descriptor(&new_fd)?;
        HostDescriptor::rename_at(
            &mut self.wasi_filesystem_view(),
            fd,
            old_path,
            new_fd,
            new_path,
        )
        .await
    }

    async fn symlink_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        src_path: String,
        dest_path: String,
    ) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        HostDescriptor::symlink_at(&mut self.wasi_filesystem_view(), fd, src_path, dest_path).await
    }

    async fn unlink_file_at_resolved(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
    ) -> FsResult<()> {
        self.require_native_descriptor(&fd)?;
        HostDescriptor::unlink_file_at(&mut self.wasi_filesystem_view(), fd, path).await
    }
}

// Retain granted descriptors even after the guest closes its copies.
// Routing `..` can switch grants, but can never manufacture a new host grant.
#[derive(Clone)]
pub(crate) enum GrantedDescriptor {
    Native(types::Descriptor),
    Virtual(Mount),
}

struct ResolvedGuestPath {
    descriptor: GrantedDescriptor,
    relative_path: String,
    guest_path: String,
}

impl RuntimeState {
    fn authorize_guest_path(&self, guest_path: &str, write: bool) -> FsResult<()> {
        let host = self
            .access
            .resolve_guest_path_to_host_path(guest_path)
            .ok_or(ErrorCode::NotPermitted)?;
        self.access
            .authorize(&host, write)
            .map_err(|_| ErrorCode::NotPermitted.into())
    }

    fn authorize_descriptor(&self, fd: &Resource<types::Descriptor>, write: bool) -> FsResult<()> {
        let guest_path = self
            .descriptor_paths
            .get(&fd.rep())
            .ok_or(ErrorCode::NotPermitted)?;
        self.authorize_guest_path(guest_path, write)
    }

    fn authorize_relative_guest_path(
        &self,
        fd: &Resource<types::Descriptor>,
        path: &str,
        write: bool,
    ) -> FsResult<()> {
        let base = self
            .descriptor_paths
            .get(&fd.rep())
            .ok_or(ErrorCode::NotPermitted)?;
        if path.starts_with('/') || path.contains('\\') {
            return Err(ErrorCode::NotPermitted.into());
        }
        let guest_path =
            normalize_guest_path(&format!("{base}/{path}")).map_err(|_| ErrorCode::NotPermitted)?;
        self.authorize_guest_path(&guest_path, write)
    }

    fn resolve_guest_path(
        &self,
        fd: &Resource<types::Descriptor>,
        path: &str,
    ) -> FsResult<Option<ResolvedGuestPath>> {
        let base = self
            .descriptor_paths
            .get(&fd.rep())
            .ok_or(ErrorCode::NotPermitted)?;
        if path.starts_with('/') || path.contains('\\') {
            return Err(ErrorCode::NotPermitted.into());
        }
        let guest_path =
            normalize_guest_path(&format!("{base}/{path}")).map_err(|_| ErrorCode::NotPermitted)?;
        let (grant_guest_path, descriptor) = self
            .granted_descriptors
            .iter()
            .filter(|(p, _)| {
                guest_path == *p || guest_path.starts_with(&format!("{}/", p.trim_end_matches('/')))
            })
            .max_by_key(|(p, r)| (r.writable(), p.len()))
            .ok_or(ErrorCode::NotPermitted)?;
        let relative_path = guest_path
            .strip_prefix(grant_guest_path)
            .unwrap()
            .trim_start_matches('/');
        Ok(Some(ResolvedGuestPath {
            descriptor: descriptor.clone(),
            relative_path: if relative_path.is_empty() {
                ".".into()
            } else {
                relative_path.into()
            },
            guest_path,
        }))
    }

    fn open_resolved_descriptor(
        &mut self,
        fd: Resource<types::Descriptor>,
        path: String,
        resolved_path: Option<ResolvedGuestPath>,
    ) -> FsResult<(
        Resource<types::Descriptor>,
        String,
        Option<u32>,
        Option<String>,
    )> {
        let Some(resolved_path) = resolved_path else {
            return Ok((fd, path, None, None));
        };
        let fd = match resolved_path.descriptor {
            GrantedDescriptor::Native(types::Descriptor::Dir(d)) => {
                let descriptor = cap_std::fs::Dir::from_std_file(d.dir.try_clone()?);
                let (parent, name) =
                    crate::access::open_parent_directory_without_symlinks(&descriptor, &resolved_path.relative_path)
                        .map_err(|_| ErrorCode::NotPermitted)?;
                let descriptor = types::Descriptor::Dir(wasmtime_wasi::filesystem::Dir::new(
                    parent.into_std_file(),
                    d.perms,
                    d.open_mode,
                    false,
                ));
                let fd = self.table.push(descriptor)?;
                let rep = fd.rep();
                return Ok((fd, name, Some(rep), Some(resolved_path.guest_path)));
            }
            GrantedDescriptor::Native(d) => self.table.push(d)?,
            GrantedDescriptor::Virtual(m) => Resource::new_own(self.table.push(m)?.rep()),
        };
        let rep = fd.rep();
        Ok((
            fd,
            resolved_path.relative_path,
            Some(rep),
            Some(resolved_path.guest_path),
        ))
    }
}

impl GrantedDescriptor {
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
