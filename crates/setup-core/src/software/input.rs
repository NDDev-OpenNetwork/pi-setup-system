//! A held artifact that returns only bytes bound to the compiled whole-file hash.

use std::{
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    time::{Duration, Instant},
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, File, Metadata, OpenOptions};
use sha2::{Digest, Sha256};

use super::Artifact;
use crate::{Error, ReasonCode, Result, digest};

const BLOCK_SIZE: usize = 64 * 1024;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const READ_BUDGET: Duration = Duration::from_secs(20);

fn unavailable(source: io::Error) -> Error {
    Error::new(
        ReasonCode::StateUnavailable,
        "software artifact is not a stable, bounded regular file",
    )
    .with_source(source)
}

fn mismatch() -> Error {
    Error::new(
        ReasonCode::IntegrityMismatch,
        "software artifact differs from the compiled length or SHA-256",
    )
}

fn regular(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    metadata.is_file()
}

fn budget(started: Instant) -> io::Result<()> {
    if started.elapsed() > READ_BUDGET {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "software artifact read budget exceeded",
        ));
    }
    Ok(())
}

/// An exact artifact retained across validation and extraction.
///
/// Opening verifies the complete SHA-256 and records hashes of those same
/// blocks. Every block returned during extraction must match, including after
/// ZIP seeks. A replaced pathname cannot select a different file, and in-place
/// writes cannot supply unchecked payload bytes. Only one block is buffered.
pub struct VerifiedArtifact {
    reading: Reading,
    metadata: Metadata,
    expected: &'static str,
}

impl VerifiedArtifact {
    /// Open and verify a regular file without following the final link.
    ///
    /// Artifacts are bounded to 8 GiB. Initial verification and extraction each
    /// have a 20-second read budget; the OS may still block one regular-file
    /// read. No partial digest is accepted.
    ///
    /// # Errors
    /// Refuses links, special or changing files, mismatched content and I/O errors.
    pub fn open(artifact: &Artifact, path: &Path) -> Result<Self> {
        let name = path.file_name().ok_or_else(mismatch)?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let directory =
            Dir::open_ambient_dir(parent, cap_std::ambient_authority()).map_err(unavailable)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No).nonblock(true);
        let mut file = directory.open_with(name, &options).map_err(unavailable)?;
        let metadata = file.metadata().map_err(unavailable)?;
        if !regular(&metadata) || artifact.bytes > MAX_BYTES {
            return Err(Error::new(
                ReasonCode::StateUnavailable,
                "software artifact is not a bounded regular file",
            ));
        }
        if metadata.len() != artifact.bytes {
            return Err(Error::new(
                ReasonCode::IntegrityMismatch,
                format!(
                    "software artifact is {} bytes; the plan named {}",
                    metadata.len(),
                    artifact.bytes
                ),
            ));
        }
        let blocks = verify_blocks(&mut file, artifact)?;
        let input = Self {
            reading: Reading {
                file,
                length: artifact.bytes,
                position: 0,
                blocks,
                buffer: Vec::new(),
                cached: None,
                started: Instant::now(),
            },
            metadata,
            expected: artifact.sha256,
        };
        input.stable()?;
        Ok(input)
    }

    pub(super) fn begin(&mut self, artifact: &Artifact) -> Result<()> {
        if self.expected != artifact.sha256 || self.reading.length != artifact.bytes {
            return Err(mismatch());
        }
        self.stable()?;
        self.reading.position = 0;
        self.reading.cached = None;
        self.reading.started = Instant::now();
        Ok(())
    }

    pub(super) fn reader(&mut self) -> &mut (impl Read + Seek) {
        &mut self.reading
    }

    pub(super) fn finish(self) -> Result<()> {
        self.stable()
    }

    fn stable(&self) -> Result<()> {
        let current = self.reading.file.metadata().map_err(unavailable)?;
        if !regular(&current)
            || current.len() != self.metadata.len()
            || current.modified().map_err(unavailable)?
                != self.metadata.modified().map_err(unavailable)?
        {
            return Err(mismatch());
        }
        Ok(())
    }
}

fn verify_blocks(file: &mut File, artifact: &Artifact) -> Result<Vec<[u8; 32]>> {
    let started = Instant::now();
    let mut buffer = vec![0; BLOCK_SIZE];
    let mut remaining = artifact.bytes;
    let mut blocks = Vec::new();
    let mut hash = Sha256::new();
    while remaining > 0 {
        budget(started).map_err(unavailable)?;
        let length = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(BLOCK_SIZE);
        let block = &mut buffer[..length];
        file.read_exact(block).map_err(unavailable)?;
        hash.update(&*block);
        blocks.push(Sha256::digest(&*block).into());
        remaining -= length as u64;
    }
    let mut extra = [0];
    if file.read(&mut extra).map_err(unavailable)? != 0 {
        return Err(mismatch());
    }
    let actual = format!("sha256:{}", digest::hex(&hash.finalize()));
    if actual != artifact.sha256 {
        return Err(Error::new(
            ReasonCode::IntegrityMismatch,
            format!(
                "software artifact hashes to {actual}; the plan named {}",
                artifact.sha256
            ),
        ));
    }
    Ok(blocks)
}

struct Reading {
    file: File,
    length: u64,
    position: u64,
    blocks: Vec<[u8; 32]>,
    buffer: Vec<u8>,
    cached: Option<u64>,
    started: Instant,
}

impl Read for Reading {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        if destination.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        budget(self.started)?;
        let block = self.position / BLOCK_SIZE as u64;
        if self.cached != Some(block) {
            self.cached = None;
            let offset = block * BLOCK_SIZE as u64;
            let length = usize::try_from(self.length - offset)
                .unwrap_or(usize::MAX)
                .min(BLOCK_SIZE);
            self.buffer.resize(length, 0);
            self.file.seek(SeekFrom::Start(offset))?;
            self.file.read_exact(&mut self.buffer)?;
            let hash: [u8; 32] = Sha256::digest(&self.buffer).into();
            let index = usize::try_from(block).map_err(io::Error::other)?;
            if self.blocks.get(index) != Some(&hash) {
                return Err(io::Error::other(
                    "software artifact block changed after verification",
                ));
            }
            self.cached = Some(block);
        }
        let offset =
            usize::try_from(self.position % BLOCK_SIZE as u64).map_err(io::Error::other)?;
        let length = destination.len().min(self.buffer.len() - offset);
        destination[..length].copy_from_slice(&self.buffer[offset..offset + length]);
        self.position += length as u64;
        Ok(length)
    }
}

impl Seek for Reading {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.position = match position {
            SeekFrom::Start(value) => Some(value),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self.length.checked_add_signed(offset),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid artifact seek"))?;
        Ok(self.position)
    }
}
