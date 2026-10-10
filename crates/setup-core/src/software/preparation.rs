//! Durable launch preparation, sealed before any destination is replaced.

use std::{
    io::{Cursor, Read, Seek, Write},
    path::Path,
    time::{Duration, Instant},
};

use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, Metadata, OpenOptions};
use serde::{Deserialize, Serialize};

use super::{launch, records};
use crate::{Result, digest};
use records::{Identity, io, leaf, open_root, refuse, sync};

const LIMIT: usize = 32 * 1024;
const PAYLOAD_LIMIT: u64 = 8 * 1024 * 1024 * 1024;

pub(super) fn journal(command: &str) -> String {
    format!(".nddev-software-{command}.exposure.json")
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Expected {
    length: u64,
    content: String,
    link: bool,
    mode: u32,
}

impl Expected {
    pub(super) fn bytes(bytes: &[u8]) -> Self {
        Self::file(bytes.len() as u64, digest::of_bytes(bytes))
    }

    pub(super) fn file(length: u64, content: String) -> Self {
        Self {
            length,
            content,
            link: false,
            mode: if cfg!(unix) { 0o600 } else { 0 },
        }
    }

    #[cfg(unix)]
    pub(super) fn link(path: &Path) -> Result<Self> {
        let content = path.to_str().ok_or_else(refuse)?.to_owned();
        Ok(Self {
            length: content.len() as u64,
            content,
            link: true,
            mode: 0o777,
        })
    }

    pub(super) fn matches(&self, stamp: &launch::Stamp) -> bool {
        stamp.matches_payload(self.length, &self.content, self.link, self.mode)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub(super) root_identity: Identity,
    pub(super) bin_identity: Identity,
    pub(super) predecessor_digest: String,
    pub(super) version: String,
    pub(super) member: String,
    pub(super) names: [String; 3],
    pub(super) expected: [Expected; 3],
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
    mode: u32,
}

impl FileIdentity {
    fn of(metadata: &Metadata) -> Result<Self> {
        if !metadata.is_file() || metadata.is_symlink() || metadata.nlink() != 1 {
            return Err(refuse());
        }
        #[cfg(unix)]
        let mode = {
            use cap_std::fs::PermissionsExt;
            metadata.permissions().mode() & 0o7777
        };
        #[cfg(not(unix))]
        let mode = {
            use cap_std::fs::MetadataExt as _;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(refuse());
            }
            u32::from(metadata.permissions().readonly())
        };
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode,
        })
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    temporary: String,
    identity: Option<FileIdentity>,
    sealed: Option<launch::Stamp>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema_version: u32,
    binding: Binding,
    entries: [Entry; 3],
}

pub(super) struct Preparation<'a> {
    path: &'a Path,
    root: &'a Dir,
    bin: &'a Dir,
    journal: String,
    record: Record,
}

impl<'a> Preparation<'a> {
    pub(super) fn begin(
        path: &'a Path,
        root: &'a Dir,
        bin: &'a Dir,
        command: &str,
        binding: &Binding,
    ) -> Result<Self> {
        if !leaf(command)
            || binding
                .expected
                .iter()
                .any(|value| value.length > PAYLOAD_LIMIT)
        {
            return Err(refuse());
        }
        let journal = journal(command);
        let previous: Option<Record> = records::read(root, &journal, LIMIT)?;
        let created = previous.is_none();
        let record = if let Some(record) = previous {
            record
        } else {
            let nonce = records::unique()?;
            Record {
                schema_version: 1,
                entries: std::array::from_fn(|index| Entry {
                    temporary: format!(".{command}.incoming-{nonce}-{index}"),
                    identity: None,
                    sealed: None,
                }),
                binding: binding.clone(),
            }
        };
        if record.schema_version != 1 || &record.binding != binding {
            return Err(refuse());
        }
        for (index, entry) in record.entries.iter().enumerate() {
            if !leaf(&entry.temporary)
                || !entry
                    .temporary
                    .starts_with(&format!(".{command}.incoming-"))
                || record.entries[..index]
                    .iter()
                    .any(|other| other.temporary == entry.temporary)
                || record.binding.names.contains(&entry.temporary)
                || (record.binding.expected[index].link && entry.identity.is_some())
                || (!record.binding.expected[index].link
                    && entry.sealed.is_some()
                    && entry.identity.is_none())
                || entry.sealed.as_ref().is_some_and(|stamp| {
                    entry.identity.is_some_and(|identity| {
                        !stamp.has_identity((identity.device, identity.inode))
                    })
                })
                || entry
                    .sealed
                    .as_ref()
                    .is_some_and(|stamp| !record.binding.expected[index].matches(stamp))
            {
                return Err(refuse());
            }
        }
        let preparation = Self {
            path,
            root,
            bin,
            journal,
            record,
        };
        preparation.revalidate()?;
        if created {
            // These exact names are durable before any temporary is created.
            preparation.save()?;
        }
        Ok(preparation)
    }

    fn revalidate(&self) -> Result<()> {
        let root = open_root(self.path)?;
        let bin = records::present(&root, "bin")?.ok_or_else(refuse)?;
        if Identity::of(&root)? != self.record.binding.root_identity
            || Identity::of(&bin)? != self.record.binding.bin_identity
            || Identity::of(self.root)? != self.record.binding.root_identity
            || Identity::of(self.bin)? != self.record.binding.bin_identity
        {
            return Err(refuse());
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        self.revalidate()?;
        records::write(self.root, &self.journal, &self.record, LIMIT)
    }

    fn ready(&self, index: usize) -> Result<bool> {
        self.revalidate()?;
        let entry = &self.record.entries[index];
        let Some(sealed) = &entry.sealed else {
            return Ok(false);
        };
        match launch::stamp(self.bin, &entry.temporary, PAYLOAD_LIMIT)? {
            Some(found) if &found == sealed => Ok(true),
            None if launch::stamp(self.bin, &self.record.binding.names[index], PAYLOAD_LIMIT)?
                .as_ref()
                == Some(sealed) =>
            {
                Ok(true)
            }
            _ => Err(refuse()),
        }
    }

    pub(super) fn bytes(&mut self, index: usize, bytes: &[u8]) -> Result<()> {
        if self.record.binding.expected[index] != Expected::bytes(bytes) {
            return Err(refuse());
        }
        self.regular(index, &mut Cursor::new(bytes))
    }

    fn open_partial(&mut self, index: usize, length: u64) -> Result<cap_std::fs::File> {
        let name = self.record.entries[index].temporary.clone();
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No)
            .nonblock(true);
        let file = if self.record.entries[index].identity.is_some() {
            options.create_new(false);
            // A recorded but missing partial is a conflict, not a fresh create.
            io(self.bin.open_with(&name, &options))?
        } else {
            match self.bin.open_with(&name, &options) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    options.create_new(false);
                    io(self.bin.open_with(&name, &options))?
                }
                Err(error) => return Err(refuse().with_source(error)),
            }
        };
        let initial = io(file.metadata())?;
        let initial_identity = FileIdentity::of(&initial)?;
        if FileIdentity::of(&io(self.bin.symlink_metadata(&name))?)? != initial_identity {
            return Err(refuse());
        }
        if self.record.entries[index].identity.is_none() {
            if initial.len() != 0 {
                return Err(refuse());
            }
            #[cfg(unix)]
            {
                use cap_std::fs::PermissionsExt;
                io(file.set_permissions(cap_std::fs::Permissions::from_mode(0o600)))?;
            }
        }
        let before = io(file.metadata())?;
        let identity = FileIdentity::of(&before)?;
        if identity.device != initial_identity.device || identity.inode != initial_identity.inode {
            return Err(refuse());
        }
        if FileIdentity::of(&io(self.bin.symlink_metadata(&name))?)? != identity
            || before.len() > length
        {
            return Err(refuse());
        }
        if let Some(recorded) = self.record.entries[index].identity {
            if identity != recorded {
                return Err(refuse());
            }
        } else {
            // Creation may precede its identity record, but never its first byte.
            if before.len() != 0 {
                return Err(refuse());
            }
            self.record.entries[index].identity = Some(identity);
            sync(self.bin)?;
            self.save()?;
        }
        Ok(file)
    }

    pub(super) fn regular(&mut self, index: usize, source: &mut (impl Read + Seek)) -> Result<()> {
        if self.ready(index)? {
            return Ok(());
        }
        let expected = &self.record.binding.expected[index];
        if expected.link {
            return Err(refuse());
        }
        let length = expected.length;
        let mut file = self.open_partial(index, length)?;
        let name = &self.record.entries[index].temporary;
        let before = io(file.metadata())?;
        let identity = FileIdentity::of(&before)?;
        io(source.rewind())?;
        let mut position = 0;
        let mut incoming = [0; 16 * 1024];
        let mut retained = [0; 16 * 1024];
        let started = Instant::now();
        while position < before.len() {
            if started.elapsed() > Duration::from_secs(20) {
                return Err(refuse());
            }
            let count = usize::try_from((before.len() - position).min(incoming.len() as u64))
                .map_err(|_| refuse())?;
            io(source.read_exact(&mut incoming[..count]))?;
            io(file.read_exact(&mut retained[..count]))?;
            if incoming[..count] != retained[..count] {
                return Err(refuse());
            }
            position += count as u64;
        }
        // Compare the complete retained prefix before appending any byte.
        self.revalidate()?;
        if FileIdentity::of(&io(file.metadata())?)? != identity
            || FileIdentity::of(&io(self.bin.symlink_metadata(name))?)? != identity
            || io(file.metadata())?.len() != before.len()
        {
            return Err(refuse());
        }
        while position < length {
            if started.elapsed() > Duration::from_secs(20) {
                return Err(refuse());
            }
            let count = usize::try_from((length - position).min(incoming.len() as u64))
                .map_err(|_| refuse())?;
            io(source.read_exact(&mut incoming[..count]))?;
            io(file.write_all(&incoming[..count]))?;
            position += count as u64;
        }
        if io(source.read(&mut incoming[..1]))? != 0 {
            return Err(refuse());
        }
        io(file.sync_all())?;
        if FileIdentity::of(&io(file.metadata())?)? != identity
            || FileIdentity::of(&io(self.bin.symlink_metadata(name))?)? != identity
        {
            return Err(refuse());
        }
        drop(file);
        self.seal(index)
    }

    #[cfg(unix)]
    pub(super) fn link(&mut self, index: usize, executable: &Path) -> Result<()> {
        if self.record.binding.expected[index] != Expected::link(executable)? {
            return Err(refuse());
        }
        if self.ready(index)? {
            return Ok(());
        }
        let name = &self.record.entries[index].temporary;
        match self.bin.symlink_contents(executable, name) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // The durable name and exact link text precede the atomic syscall.
                if !io(self.bin.symlink_metadata(name))?.is_symlink()
                    || io(self.bin.read_link_contents(name))? != executable
                {
                    return Err(refuse());
                }
            }
            Err(error) => return Err(refuse().with_source(error)),
        }
        self.seal(index)
    }

    fn seal(&mut self, index: usize) -> Result<()> {
        let entry = &self.record.entries[index];
        let stamp = launch::stamp(self.bin, &entry.temporary, PAYLOAD_LIMIT)?.ok_or_else(refuse)?;
        if !self.record.binding.expected[index].matches(&stamp)
            || entry
                .identity
                .is_some_and(|identity| !stamp.has_identity((identity.device, identity.inode)))
        {
            return Err(refuse());
        }
        sync(self.bin)?;
        self.record.entries[index].sealed = Some(stamp);
        self.save()
    }

    pub(super) fn commit(self, predecessor: &launch::Snapshot) -> Result<()> {
        for (index, entry) in self.record.entries.iter().enumerate() {
            self.ready(index)?;
            predecessor.check_replacement(
                self.bin,
                index,
                entry.sealed.as_ref().ok_or_else(refuse)?,
            )?;
        }
        for (index, entry) in self.record.entries.iter().enumerate() {
            self.revalidate()?;
            let stamp = entry.sealed.as_ref().ok_or_else(refuse)?;
            self.ready(index)?;
            predecessor.check_replacement(self.bin, index, stamp)?;
            if launch::stamp(self.bin, &entry.temporary, PAYLOAD_LIMIT)?.is_some() {
                io(self.bin.rename(
                    &entry.temporary,
                    self.bin,
                    &self.record.binding.names[index],
                ))?;
                sync(self.bin)?;
            }
        }
        self.revalidate()?;
        for (index, entry) in self.record.entries.iter().enumerate() {
            if launch::stamp(self.bin, &entry.temporary, PAYLOAD_LIMIT)?.is_some()
                || launch::stamp(self.bin, &self.record.binding.names[index], PAYLOAD_LIMIT)?
                    != entry.sealed
            {
                return Err(refuse());
            }
        }
        io(self.root.remove_file(&self.journal))?;
        sync(self.root)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use std::fs;

    #[test]
    fn interrupted_preparation_resumes_exact_entries_and_preserves_conflicts() {
        let path = std::env::temp_dir().join(format!(
            "software-preparation-{}",
            records::unique().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        let predecessor = launch::Snapshot::prepare(&path, "tool", "tool").unwrap();
        let root = open_root(&path).unwrap();
        let bin = records::present(&root, "bin").unwrap().unwrap();
        let values: [&[u8]; 3] = [b"program", b"1.2.3", b"manifest"];
        let binding = Binding {
            root_identity: Identity::of(&root).unwrap(),
            bin_identity: Identity::of(&bin).unwrap(),
            predecessor_digest: digest::of_bytes(&serde_json::to_vec(&predecessor).unwrap()),
            version: "1.2.3".into(),
            member: "tool".into(),
            names: [
                "tool".into(),
                ".tool.version".into(),
                ".tool.manifest.json".into(),
            ],
            expected: values.map(Expected::bytes),
        };
        let begin = || Preparation::begin(&path, &root, &bin, "tool", &binding).unwrap();
        let mut preparation = begin();
        let temporary = preparation
            .record
            .entries
            .each_ref()
            .map(|entry| path.join("bin").join(&entry.temporary));
        // The names exist durably before creation; an empty file can precede
        // the identity record, while an unrecorded nonempty file cannot.
        fs::write(&temporary[0], b"").unwrap();
        fs::write(&temporary[2], b"foreign temporary").unwrap();
        assert!(preparation.bytes(2, values[2]).is_err());
        assert_eq!(fs::read(&temporary[2]).unwrap(), b"foreign temporary");
        fs::remove_file(&temporary[2]).unwrap();
        preparation.bytes(0, values[0]).unwrap();
        let retained = path.join("retained");
        fs::rename(&temporary[0], &retained).unwrap();
        fs::write(&temporary[0], b"foreign replacement").unwrap();
        drop(preparation);
        let mut preparation = begin();
        assert!(preparation.bytes(0, values[0]).is_err());
        assert_eq!(fs::read(&temporary[0]).unwrap(), b"foreign replacement");
        assert!(!path.join("bin/tool").exists());
        fs::remove_file(&temporary[0]).unwrap();
        fs::rename(&retained, &temporary[0]).unwrap();

        // Reproduce a write interrupted after durable identity, then verify
        // both a missing pathname and changed retained bytes without effects.
        fs::write(&temporary[1], b"").unwrap();
        preparation.record.entries[1].identity = Some(
            FileIdentity::of(
                &bin.symlink_metadata(&preparation.record.entries[1].temporary)
                    .unwrap(),
            )
            .unwrap(),
        );
        preparation.save().unwrap();
        fs::rename(&temporary[1], &retained).unwrap();
        assert!(preparation.bytes(1, values[1]).is_err());
        assert!(!temporary[1].exists());
        fs::rename(&retained, &temporary[1]).unwrap();
        fs::write(&temporary[1], b"bad").unwrap();
        assert!(preparation.bytes(1, values[1]).is_err());
        assert_eq!(fs::read(&temporary[1]).unwrap(), b"bad");
        fs::write(&temporary[1], b"1.2").unwrap();
        drop(preparation);
        let mut preparation = begin();
        for (index, value) in values.iter().enumerate() {
            preparation.bytes(index, value).unwrap();
        }
        fs::write(path.join("bin/tool"), b"foreign launch").unwrap();
        assert!(preparation.commit(&predecessor).is_err());
        assert_eq!(fs::read(path.join("bin/tool")).unwrap(), b"foreign launch");
        assert!(!path.join("bin/.tool.version").exists());
        assert!(temporary.iter().all(|entry| entry.is_file()));
        fs::remove_file(path.join("bin/tool")).unwrap();

        // A kill after any rename requires no new preparation and does not
        // replace an already committed physical result on the next attempt.
        fs::rename(&temporary[0], path.join("bin/tool")).unwrap();
        let mut preparation = begin();
        for (index, value) in values.iter().enumerate() {
            preparation.bytes(index, value).unwrap();
        }
        preparation.commit(&predecessor).unwrap();
        assert!(!path.join(journal("tool")).exists());
        assert!(temporary.iter().all(|entry| !entry.exists()));
        for (name, value) in binding.names.iter().zip(values) {
            assert_eq!(fs::read(path.join("bin").join(name)).unwrap(), value);
        }
        drop(bin);
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }
}
