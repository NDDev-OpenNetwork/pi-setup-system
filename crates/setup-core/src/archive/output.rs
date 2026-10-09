//! Relative extraction writes anchored to a held destination directory.

use std::{ffi::OsStr, fs::File, io, path::Path};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};

fn invalid() -> io::Error {
    io::Error::other("archive destination is not a plain directory or new regular file")
}

fn component(name: &OsStr) -> io::Result<()> {
    let name = name.to_str().ok_or_else(invalid)?;
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
        || device
    {
        return Err(invalid());
    }
    Ok(())
}

fn reparse(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

fn child(parent: &Dir, name: &Path) -> io::Result<Dir> {
    match parent.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let directory = parent.open_dir_nofollow(name)?;
    if reparse(&directory.dir_metadata()?) {
        return Err(invalid());
    }
    Ok(directory)
}

pub(super) struct Destination {
    root: Dir,
}

impl Destination {
    /// The caller selects the parent; the final destination and every archive
    /// component below it are opened without following links. Missing parents
    /// are created from the nearest existing ancestor, retaining each handle.
    pub(super) fn open(path: &Path) -> io::Result<Self> {
        let Some(name) = path.file_name() else {
            if path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(invalid());
            }
            return Ok(Self {
                root: Dir::open_ambient_dir(path, cap_std::ambient_authority())?,
            });
        };
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = match Dir::open_ambient_dir(parent, cap_std::ambient_authority()) {
            Ok(parent) => parent,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::open(parent)?.root,
            Err(error) => return Err(error),
        };
        Ok(Self {
            root: child(&parent, Path::new(name))?,
        })
    }

    pub(super) fn directory(&self, relative: &Path) -> io::Result<Dir> {
        let mut directory = self.root.try_clone()?;
        for part in relative.components() {
            match part {
                std::path::Component::Normal(name) => {
                    component(name)?;
                    directory = child(&directory, Path::new(name))?;
                }
                std::path::Component::CurDir => {}
                _ => return Err(invalid()),
            }
        }
        Ok(directory)
    }

    pub(super) fn file(&self, relative: &Path) -> io::Result<File> {
        let name = relative.file_name().ok_or_else(invalid)?;
        component(name)?;
        let directory = self.directory(relative.parent().unwrap_or_else(|| Path::new("")))?;
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No)
            .nonblock(true);
        let file = directory.open_with(name, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || reparse(&metadata) {
            return Err(invalid());
        }
        Ok(file.into_std())
    }
}
