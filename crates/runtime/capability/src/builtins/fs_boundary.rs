//! Descriptor-anchored filesystem operations for path capabilities.
//!
//! Policy checks over `PathBuf`s are useful for diagnostics, but are not an
//! authority boundary: a local actor can replace a directory or a symlink
//! between a canonicalization and a later open.  This module keeps the final
//! operation anchored to an opened root directory and refuses symlink
//! components.  The capability callers intentionally use this module for both
//! reads and writes, including their size checks.

use std::io;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use rustix::fs::{AtFlags, Mode, OFlags, linkat, mkdirat, open, openat, renameat, unlinkat};
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::{Read, Write};

#[cfg(unix)]
fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir().map(|cwd| cwd.join(path))
    }
}

#[cfg(unix)]
fn relative_components(root: &Path, path: &Path) -> io::Result<Vec<std::ffi::OsString>> {
    let root_absolute = absolute_path(root)?;
    let root_canonical = std::fs::canonicalize(root)?;
    let path = absolute_path(path)?;
    // Prefer the configured spelling so a harmless system alias (for example
    // macOS `/var` -> `/private/var`) does not change the names we pass to
    // openat. Fall back to the canonical spelling for callers that already
    // resolved the root before reaching this boundary.
    let relative = path
        .strip_prefix(&root_absolute)
        .or_else(|_| path.strip_prefix(&root_canonical))
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "requested path is outside the configured filesystem root",
            )
        })?;
    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => components.push(name.to_os_string()),
            // `normalize_path_lexically` should already remove these, but the
            // boundary rejects them independently so callers cannot bypass it.
            Component::CurDir | Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "relative path contains a traversal component",
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "absolute path is not relative to the configured root",
                ));
            }
        }
    }
    if components.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a regular file path is required",
        ));
    }
    Ok(components)
}

#[cfg(unix)]
fn open_root(root: &Path) -> io::Result<File> {
    // Canonicalization makes the root identity explicit; opening with
    // O_NOFOLLOW still prevents a final replacement from turning it into a
    // symlink after that check.
    let canonical = std::fs::canonicalize(root)?;
    let fd = open(
        canonical,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))?;
    Ok(fd.into())
}

#[cfg(unix)]
fn open_dir_at(parent: &File, name: &std::ffi::OsStr) -> io::Result<File> {
    let fd = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))?;
    Ok(fd.into())
}

#[cfg(unix)]
fn open_parent(
    root: &File,
    components: &[std::ffi::OsString],
    create: bool,
) -> io::Result<(File, std::ffi::OsString)> {
    let mut directory = root.try_clone()?;
    for name in &components[..components.len() - 1] {
        match open_dir_at(&directory, name) {
            Ok(next) => directory = next,
            Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
                if let Err(mkdir_error) = mkdirat(&directory, name, Mode::from(0o700)) {
                    let mkdir_error = io::Error::from_raw_os_error(mkdir_error.raw_os_error());
                    if mkdir_error.kind() != io::ErrorKind::AlreadyExists {
                        return Err(mkdir_error);
                    }
                }
                directory = open_dir_at(&directory, name)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok((
        directory,
        components.last().expect("non-empty components").clone(),
    ))
}

#[cfg(unix)]
fn open_file_at(
    parent: &File,
    name: &std::ffi::OsStr,
    flags: OFlags,
    mode: Mode,
) -> io::Result<File> {
    let fd = openat(
        parent,
        name,
        flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        mode,
    )
    .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))?;
    Ok(fd.into())
}

#[cfg(unix)]
fn require_regular(file: &File) -> io::Result<u64> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "filesystem capability target is not a regular file",
        ));
    }
    Ok(metadata.len())
}

#[cfg(unix)]
fn read_at(root: &Path, path: &Path, max: usize) -> io::Result<Vec<u8>> {
    let components = relative_components(root, path)?;
    let root_fd = open_root(root)?;
    let (parent, name) = open_parent(&root_fd, &components, false)?;
    let file = open_file_at(&parent, &name, OFlags::RDONLY, Mode::empty())?;
    let size = require_regular(&file)?;
    let bounded = u64::try_from(max).unwrap_or(u64::MAX);
    let mut bytes = Vec::with_capacity(usize::try_from(size.min(bounded)).unwrap_or(max));
    // Reading max+1 lets the caller distinguish an exact-limit file from a
    // file that grew after policy inspection without allocating unboundedly.
    file.take(bounded.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > max {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "filesystem capability read exceeds configured size limit",
        ));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn write_at(
    root: &Path,
    path: &Path,
    content: &[u8],
    append: bool,
    create_directories: bool,
    overwrite_existing: bool,
    max: Option<usize>,
) -> io::Result<()> {
    if !append && max.is_some_and(|limit| content.len() > limit) {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "filesystem capability write exceeds configured size limit",
        ));
    }
    let components = relative_components(root, path)?;
    let root_fd = open_root(root)?;
    let (parent, name) = open_parent(&root_fd, &components, create_directories)?;

    if append {
        let file = open_file_at(
            &parent,
            &name,
            OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE,
            Mode::from(0o600),
        )?;
        let old_size = require_regular(&file)?;
        if max.is_some_and(|limit| {
            old_size > u64::try_from(limit).unwrap_or(u64::MAX)
                || old_size.saturating_add(content.len() as u64)
                    > u64::try_from(limit).unwrap_or(u64::MAX)
        }) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "filesystem capability append exceeds configured size limit",
            ));
        }
        (&file).write_all(content)?;
        return Ok(());
    }

    // Write a private temporary file in the already-open parent directory.
    // A rename of this descriptor-anchored name cannot follow a target symlink
    // or escape the configured root.
    let temporary = std::ffi::OsString::from(format!(".apxm_tmp_{}", uuid::Uuid::now_v7()));
    let file = open_file_at(
        &parent,
        &temporary,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
        Mode::from(0o600),
    )?;
    (&file).write_all(content)?;
    file.sync_all()?;
    drop(file);

    let result = if overwrite_existing {
        renameat(&parent, &temporary, &parent, &name)
            .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))
    } else {
        // linkat is the portable Unix equivalent of a no-replace publish:
        // EEXIST is atomic and the destination is never followed.
        linkat(&parent, &temporary, &parent, &name, AtFlags::empty())
            .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))
            .and_then(|()| {
                unlinkat(&parent, &temporary, AtFlags::empty())
                    .map_err(|error| io::Error::from_raw_os_error(error.raw_os_error()))
            })
    };
    if result.is_err() {
        let _ = unlinkat(&parent, &temporary, AtFlags::empty());
    }
    result
}

#[cfg(unix)]
pub(crate) fn secure_read_under_root(root: &Path, path: &Path, max: usize) -> io::Result<Vec<u8>> {
    read_at(root, path, max)
}

#[cfg(unix)]
pub(crate) fn secure_write_under_root(
    root: &Path,
    path: &Path,
    content: &[u8],
    append: bool,
    create_directories: bool,
    overwrite_existing: bool,
    max: Option<usize>,
) -> io::Result<()> {
    write_at(
        root,
        path,
        content,
        append,
        create_directories,
        overwrite_existing,
        max,
    )
}

#[cfg(not(unix))]
pub(crate) fn secure_read_under_root(
    _root: &Path,
    _path: &Path,
    _max: usize,
) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor-anchored filesystem confinement is unavailable on this platform",
    ))
}

#[cfg(not(unix))]
pub(crate) fn secure_write_under_root(
    _root: &Path,
    _path: &Path,
    _content: &[u8],
    _append: bool,
    _create_directories: bool,
    _overwrite_existing: bool,
    _max: Option<usize>,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor-anchored filesystem confinement is unavailable on this platform",
    ))
}
