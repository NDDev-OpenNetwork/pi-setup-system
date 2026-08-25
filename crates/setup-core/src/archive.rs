//! Reading the one archive shape every vendor ships.
//!
//! Six of the seven products distribute their binary as a gzip-compressed tar,
//! and the seventh publishes the executable as plain bytes. Nothing else
//! appears: no zip, no zstd, no brotli. That is measured rather than assumed --
//! every artifact named in a baseline was fetched and its raw headers read.
//!
//! The measurement mattered, because the convenient tools lie about it. Python's
//! `tarfile` consumes GNU long-name headers transparently and reports only the
//! members, so an archive that needs them looks identical to one that does not.
//! Reading `typeflag` off the raw 512-byte blocks says what is really there:
//!
//! ```text
//! claude       '0' x4                          ustar\0
//! codex        '0' x8                          ustar\0
//! opencode     '0' x2                          ustar\0
//! antigravity  '0' x1                          ustar␠␠
//! cursor       '0' x442  '5' x127  'L' x52     ustar␠␠
//! ```
//!
//! So this reader accepts exactly two dialects -- POSIX `ustar` and GNU tar --
//! and exactly three entry types: a regular file, a directory, and the GNU
//! long-name header that carries a path too long for the 100-byte field. Every
//! other type flag is refused **by name**, because the reason a symlink is
//! refused is worth saying out loud: extraction that honours one can be made to
//! write through it, outside the directory the caller asked for.
//!
//! It streams. `claude`'s payload inflates to 391 MB and `cursor`'s tree holds
//! 569 entries; buffering either whole would cost more memory than the machines
//! this runs on can spare, so bytes move from the compressed input to the file
//! on disk without a copy of the archive existing anywhere.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Component, Path};

use crate::checksum::continue_crc32;
use crate::error::{Error, ReasonCode, Result};

/// How large a header block is, and the unit every tar offset is a multiple of.
const BLOCK: usize = 512;

/// What one archive entry is.
///
/// Only two kinds exist here. Anything else the format can express is refused
/// before it becomes an [`Entry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
}

/// One entry's metadata, as this reader is willing to describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The path inside the archive, already checked to be relative and safe.
    pub path: String,
    /// Whether it is a file or a directory.
    pub kind: Kind,
    /// The permission bits the archive recorded.
    pub mode: u32,
    /// The byte length of the entry's content. Zero for a directory.
    pub size: u64,
}

impl Entry {
    /// Whether the archive marked this entry executable.
    #[must_use]
    pub const fn is_executable(&self) -> bool {
        self.mode & 0o111 != 0
    }
}

fn refuse(detail: impl Into<String>) -> Error {
    Error::new(ReasonCode::IntegrityMismatch, detail)
}

fn from_io(detail: &str, error: io::Error) -> Error {
    Error::new(ReasonCode::StateUnavailable, format!("{detail}: {error}")).with_source(error)
}

/// A gzip member, inflated as it is read.
///
/// Only the framing is written here. The DEFLATE stream inside it is decoded by
/// `miniz_oxide`, which is the one part of this worth taking as a dependency:
/// an inflate loop is several hundred lines of Huffman decoding whose bugs are
/// memory-safety bugs, and it is not improved by being ours.
pub struct Gunzip<R: Read> {
    inner: R,
    state: Box<miniz_oxide::inflate::stream::InflateState>,
    input: Vec<u8>,
    filled: usize,
    consumed: usize,
    finished: bool,
    crc: u32,
    length: u64,
}

impl<R: Read> Gunzip<R> {
    /// Read the gzip header and prepare to inflate what follows.
    ///
    /// # Errors
    ///
    /// Refuses a stream that is not a gzip member, or that uses a compression
    /// method other than DEFLATE.
    pub fn new(mut inner: R) -> Result<Self> {
        let mut head = [0_u8; 10];
        inner
            .read_exact(&mut head)
            .map_err(|error| from_io("gzip header could not be read", error))?;
        if head[0] != 0x1F || head[1] != 0x8B {
            return Err(refuse(format!(
                "not a gzip member: magic {:#04x}{:02x}",
                head[0], head[1]
            )));
        }
        if head[2] != 8 {
            return Err(refuse(format!(
                "gzip compression method {} is not DEFLATE",
                head[2]
            )));
        }

        let flags = head[3];
        if flags & 0b1110_0000 != 0 {
            return Err(refuse(format!(
                "gzip reserved flag bits set: {flags:#010b}"
            )));
        }
        if flags & 0b0000_0100 != 0 {
            let mut length = [0_u8; 2];
            inner
                .read_exact(&mut length)
                .map_err(|error| from_io("gzip extra field length could not be read", error))?;
            let mut extra = vec![0_u8; usize::from(u16::from_le_bytes(length))];
            inner
                .read_exact(&mut extra)
                .map_err(|error| from_io("gzip extra field could not be read", error))?;
        }
        for (bit, what) in [(0b0000_1000_u8, "name"), (0b0001_0000, "comment")] {
            if flags & bit != 0 {
                let mut byte = [0_u8; 1];
                loop {
                    inner.read_exact(&mut byte).map_err(|error| {
                        from_io(&format!("gzip {what} field could not be read"), error)
                    })?;
                    if byte[0] == 0 {
                        break;
                    }
                }
            }
        }
        if flags & 0b0000_0010 != 0 {
            let mut check = [0_u8; 2];
            inner
                .read_exact(&mut check)
                .map_err(|error| from_io("gzip header checksum could not be read", error))?;
        }

        Ok(Self {
            inner,
            state: miniz_oxide::inflate::stream::InflateState::new_boxed(
                miniz_oxide::DataFormat::Raw,
            ),
            input: vec![0_u8; 64 * 1024],
            filled: 0,
            consumed: 0,
            finished: false,
            crc: 0,
            length: 0,
        })
    }

    /// Check the trailer against what was actually inflated.
    fn finish(&mut self) -> io::Result<()> {
        let mut trailer = [0_u8; 8];
        let mut have = self.filled - self.consumed;
        let carried = have.min(8);
        trailer[..carried].copy_from_slice(&self.input[self.consumed..self.consumed + carried]);
        self.consumed += carried;
        have = carried;
        while have < 8 {
            let read = self.inner.read(&mut trailer[have..])?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "gzip trailer is truncated",
                ));
            }
            have += read;
        }

        let stated_crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
        let stated_length = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
        if stated_crc != self.crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "gzip CRC-32 mismatch: stated {stated_crc:#010x}, inflated {:#010x}",
                    self.crc
                ),
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        let low = self.length as u32;
        if stated_length != low {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "gzip length mismatch: stated {stated_length}, inflated {}",
                    self.length
                ),
            ));
        }
        Ok(())
    }
}

impl<R: Read> Read for Gunzip<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.finished || out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.consumed == self.filled {
                self.filled = self.inner.read(&mut self.input)?;
                self.consumed = 0;
                if self.filled == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "gzip stream ended before the DEFLATE data did",
                    ));
                }
            }

            let result = miniz_oxide::inflate::stream::inflate(
                &mut self.state,
                &self.input[self.consumed..self.filled],
                out,
                miniz_oxide::MZFlush::None,
            );
            self.consumed += result.bytes_consumed;
            let written = result.bytes_written;
            if written > 0 {
                self.crc = continue_crc32(self.crc, &out[..written]);
                self.length = self.length.wrapping_add(written as u64);
            }

            match result.status {
                Ok(miniz_oxide::MZStatus::StreamEnd) => {
                    self.finished = true;
                    self.finish()?;
                    return Ok(written);
                }
                Ok(_) => {
                    if written > 0 {
                        return Ok(written);
                    }
                }
                Err(error) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("DEFLATE stream is malformed: {error:?}"),
                    ));
                }
            }
        }
    }
}

/// A tar stream, read one entry at a time.
pub struct Tar<R: Read> {
    inner: R,
    remaining: u64,
    padding: usize,
    /// Allocated once. `copy_entry` runs per entry, and cursor's tree has 569
    /// of them; a fresh 64 KB buffer each time would be 569 allocations to
    /// move the same bytes.
    buffer: Box<[u8]>,
}

impl<R: Read> Tar<R> {
    /// Wrap a reader positioned at the first header block.
    #[must_use]
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            remaining: 0,
            padding: 0,
            buffer: vec![0_u8; 64 * 1024].into_boxed_slice(),
        }
    }

    /// Advance to the next entry, skipping any content the caller did not read.
    ///
    /// Returns `None` at the end-of-archive marker.
    ///
    /// # Errors
    ///
    /// Refuses an unknown magic, a header whose checksum does not match, a path
    /// that is not safely relative, or any entry type outside file, directory
    /// and GNU long name.
    pub fn next_entry(&mut self) -> Result<Option<Entry>> {
        self.skip_rest()?;

        let mut long_name: Option<String> = None;
        loop {
            let Some(block) = self.read_block()? else {
                return Ok(None);
            };

            verify_checksum(&block)?;
            let magic = &block[257..263];
            let gnu = magic == b"ustar ";
            let posix = magic == b"ustar\0";
            if !gnu && !posix {
                return Err(refuse(format!(
                    "tar magic {:?} is neither POSIX ustar nor GNU tar",
                    String::from_utf8_lossy(magic)
                )));
            }

            let size = octal(&block[124..136], "size")?;
            let mode = u32::try_from(octal(&block[100..108], "mode")?).unwrap_or(0o644);
            self.remaining = size;
            self.padding = padding_for(size);

            let flag = block[156];
            match flag {
                b'L' => {
                    if !gnu {
                        return Err(refuse(
                            "a GNU long-name header appeared in a POSIX ustar archive",
                        ));
                    }
                    let mut raw = Vec::new();
                    self.copy_entry(&mut raw)?;
                    let text = String::from_utf8(raw)
                        .map_err(|_| refuse("a GNU long name is not valid UTF-8"))?;
                    long_name = Some(text.trim_end_matches('\0').to_owned());
                }
                b'0' | b'\0' | b'5' => {
                    let path = match long_name.take() {
                        Some(name) => name,
                        None => joined_name(&block, posix)?,
                    };
                    let kind = if flag == b'5' || path.ends_with('/') {
                        Kind::Directory
                    } else {
                        Kind::File
                    };
                    let path = check_relative(path.trim_end_matches('/'))?;
                    if kind == Kind::Directory && size != 0 {
                        return Err(refuse(format!("directory {path} carries {size} bytes")));
                    }
                    return Ok(Some(Entry {
                        path,
                        kind,
                        mode,
                        size,
                    }));
                }
                other => return Err(refuse(refusal_for(other))),
            }
        }
    }

    /// Give the wrapped reader back, so the caller can finish the stream.
    ///
    /// A tar ends at its zero-block marker, which is *not* the end of the gzip
    /// member around it. Stopping there leaves the trailer unread, and the
    /// trailer is the only thing that says whether what was inflated is what
    /// was compressed -- so the CRC would never be checked at all. A test
    /// corrupting the trailer found exactly that.
    pub fn into_inner(self) -> R {
        self.inner
    }

    /// Write the current entry's content to `out`.
    ///
    /// # Errors
    ///
    /// Fails if the archive ends early or the sink refuses the bytes.
    pub fn copy_entry(&mut self, out: &mut impl Write) -> Result<()> {
        while self.remaining > 0 {
            let want = usize::try_from(self.remaining.min(self.buffer.len() as u64)).unwrap_or(1);
            let read = self
                .inner
                .read(&mut self.buffer[..want])
                .map_err(|error| from_io("archive content could not be read", error))?;
            if read == 0 {
                return Err(refuse("archive ended in the middle of an entry"));
            }
            out.write_all(&self.buffer[..read])
                .map_err(|error| from_io("archive content could not be written", error))?;
            self.remaining -= read as u64;
        }
        self.skip_padding()
    }

    fn skip_rest(&mut self) -> Result<()> {
        let mut sink = io::sink();
        if self.remaining > 0 {
            self.copy_entry(&mut sink)
        } else {
            self.skip_padding()
        }
    }

    fn skip_padding(&mut self) -> Result<()> {
        if self.padding == 0 {
            return Ok(());
        }
        let mut waste = [0_u8; BLOCK];
        let take = self.padding;
        self.padding = 0;
        self.inner
            .read_exact(&mut waste[..take])
            .map_err(|error| from_io("archive padding could not be read", error))
    }

    /// Read one 512-byte block, returning `None` at the end-of-archive marker.
    fn read_block(&mut self) -> Result<Option<[u8; BLOCK]>> {
        let mut block = [0_u8; BLOCK];
        let mut have = 0;
        while have < BLOCK {
            let read = self
                .inner
                .read(&mut block[have..])
                .map_err(|error| from_io("archive header could not be read", error))?;
            if read == 0 {
                if have == 0 {
                    return Ok(None);
                }
                return Err(refuse("archive ended in the middle of a header"));
            }
            have += read;
        }
        if block.iter().all(|byte| *byte == 0) {
            return Ok(None);
        }
        Ok(Some(block))
    }
}

/// Why one type flag is refused, said in the terms that make it a decision.
fn refusal_for(flag: u8) -> String {
    let what = match flag {
        b'1' => "a hard link",
        b'2' => "a symbolic link",
        b'3' => "a character device",
        b'4' => "a block device",
        b'6' => "a FIFO",
        b'7' => "a contiguous file",
        b'x' | b'g' => "a pax extended header",
        b'K' => "a GNU long link name",
        _ => "an entry type outside the format this reader accepts",
    };
    format!(
        "refusing {what} (type flag {:?}): extraction here writes regular files and directories \
         and nothing else, so that no entry can redirect a later write outside the destination",
        char::from(flag)
    )
}

/// The header checksum, computed the way the format defines it.
fn verify_checksum(block: &[u8; BLOCK]) -> Result<()> {
    let stated = octal(&block[148..156], "checksum")?;
    let mut unsigned = 0_u64;
    let mut signed = 0_i64;
    for (index, byte) in block.iter().enumerate() {
        let value = if (148..156).contains(&index) {
            b' '
        } else {
            *byte
        };
        unsigned += u64::from(value);
        signed += i64::from(value.cast_signed());
    }
    if stated == unsigned || i64::try_from(stated).is_ok_and(|want| want == signed) {
        return Ok(());
    }
    Err(refuse(format!(
        "tar header checksum mismatch: stated {stated}, computed {unsigned}"
    )))
}

/// A tar numeric field: octal digits, then a NUL or a space.
fn octal(field: &[u8], what: &str) -> Result<u64> {
    let text: Vec<u8> = field
        .iter()
        .copied()
        .take_while(|byte| *byte != 0 && *byte != b' ')
        .skip_while(|byte| *byte == b' ')
        .collect();
    if text.is_empty() {
        return Ok(0);
    }
    let text = std::str::from_utf8(&text)
        .map_err(|_| refuse(format!("tar {what} field is not ASCII octal")))?;
    u64::from_str_radix(text, 8)
        .map_err(|_| refuse(format!("tar {what} field {text:?} is not octal")))
}

/// POSIX ustar splits a long path across `prefix` and `name`.
fn joined_name(block: &[u8; BLOCK], posix: bool) -> Result<String> {
    let name = field_text(&block[0..100], "name")?;
    if !posix {
        return Ok(name);
    }
    let prefix = field_text(&block[345..500], "prefix")?;
    if prefix.is_empty() {
        Ok(name)
    } else {
        Ok(format!("{prefix}/{name}"))
    }
}

fn field_text(field: &[u8], what: &str) -> Result<String> {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    std::str::from_utf8(&field[..end])
        .map(str::to_owned)
        .map_err(|_| refuse(format!("tar {what} field is not valid UTF-8")))
}

fn padding_for(size: u64) -> usize {
    let remainder = usize::try_from(size % BLOCK as u64).unwrap_or(0);
    if remainder == 0 { 0 } else { BLOCK - remainder }
}

/// Refuse any path that could name something outside the destination.
///
/// The rules are deliberately stricter than the format allows. A tar may carry
/// an absolute path, a `..`, or a Windows drive letter; honouring any of them
/// means the caller asked to fill one directory and got a write somewhere else.
fn check_relative(path: &str) -> Result<String> {
    if path.is_empty() {
        return Err(refuse("archive entry has an empty path"));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(refuse(format!("archive entry {path} is an absolute path")));
    }
    if path.contains('\\') {
        return Err(refuse(format!(
            "archive entry {path} contains a backslash, which names a different file on each system"
        )));
    }
    if path.contains('\0') {
        return Err(refuse("archive entry path contains a NUL byte"));
    }
    if path.as_bytes().get(1) == Some(&b':') {
        return Err(refuse(format!(
            "archive entry {path} carries a drive letter"
        )));
    }
    for part in path.split('/') {
        if part == ".." {
            return Err(refuse(format!(
                "archive entry {path} climbs out of the destination"
            )));
        }
    }
    Ok(path.to_owned())
}

/// How much an extraction is allowed to produce.
///
/// A plan states the compressed length it expects; it cannot state what the
/// archive will inflate to without trusting the archive. These are the caller's
/// own limits, checked as bytes arrive, so a stream that keeps producing is
/// stopped rather than filling the disk.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The largest number of entries the archive may contain.
    pub entries: u64,
    /// The largest total byte count the archive may inflate to.
    pub bytes: u64,
}

/// Extract a gzip-compressed tar into `destination`.
///
/// Returns what was written, in archive order.
///
/// # Errors
///
/// Refuses a malformed stream, an unsafe path, an entry type outside file and
/// directory, or an archive that exceeds `limits`.
pub fn extract_gzip_tar(
    source: impl Read,
    destination: &Path,
    limits: Limits,
) -> Result<Vec<Entry>> {
    let mut tar = Tar::new(Gunzip::new(source)?);
    let mut written = Vec::new();
    let mut total = 0_u64;

    fs::create_dir_all(destination)
        .map_err(|error| from_io("destination could not be created", error))?;

    while let Some(entry) = tar.next_entry()? {
        if written.len() as u64 >= limits.entries {
            return Err(refuse(format!(
                "archive holds more than the {} entries this extraction allows",
                limits.entries
            )));
        }
        total = total.saturating_add(entry.size);
        if total > limits.bytes {
            return Err(refuse(format!(
                "archive inflates past the {} bytes this extraction allows",
                limits.bytes
            )));
        }

        let path = destination.join(&entry.path);
        guard_within(destination, &path)?;
        match entry.kind {
            Kind::Directory => {
                fs::create_dir_all(&path)
                    .map_err(|error| from_io("archive directory could not be created", error))?;
            }
            Kind::File => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        from_io("archive parent directory could not be created", error)
                    })?;
                }
                let mut file = fs::File::create(&path)
                    .map_err(|error| from_io("archive file could not be created", error))?;
                tar.copy_entry(&mut file)?;
                file.sync_all()
                    .map_err(|error| from_io("archive file could not be flushed", error))?;
                apply_mode(&path, entry.mode)?;
            }
        }
        written.push(entry);
    }

    // Read past the tar's end-of-archive marker to the end of the gzip member,
    // so its trailer is reached and its CRC-32 and length are checked. What
    // remains is padding, so this costs nothing and is the only place the
    // compression's own integrity check happens.
    let mut rest = tar.into_inner();
    io::copy(&mut rest, &mut io::sink())
        .map_err(|error| refuse(format!("archive did not end cleanly: {error}")))?;

    Ok(written)
}

/// Prove the joined path really is inside the destination.
///
/// [`check_relative`] rejects the paths that could escape, and this repeats the
/// question against the assembled path. Two checks rather than one because the
/// cost is a string comparison and the failure they prevent is a write outside
/// the directory the caller named.
fn guard_within(destination: &Path, candidate: &Path) -> Result<()> {
    let escapes = candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir));
    if escapes || !candidate.starts_with(destination) {
        return Err(refuse(format!(
            "{} is not inside {}",
            candidate.display(),
            destination.display()
        )));
    }
    Ok(())
}

/// Carry the archive's executable bit across, where the system has one.
#[cfg(unix)]
fn apply_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let bits = if mode & 0o111 == 0 { 0o644 } else { 0o755 };
    fs::set_permissions(path, fs::Permissions::from_mode(bits))
        .map_err(|error| from_io("archive file permissions could not be set", error))
}

/// Windows has no mode bits to carry, and executability is decided by extension.
#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Place plain bytes as an executable.
///
/// Grok publishes its binary directly rather than inside an archive, so this is
/// the whole of its extraction: the artifact *is* the program.
///
/// # Errors
///
/// Fails if the destination cannot be created or written.
pub fn place_executable(source: impl Read, destination: &Path) -> Result<u64> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| from_io("destination directory could not be created", error))?;
    }
    let mut file = fs::File::create(destination)
        .map_err(|error| from_io("executable could not be created", error))?;
    let mut source = source;
    let written = io::copy(&mut source, &mut file)
        .map_err(|error| from_io("executable could not be written", error))?;
    file.sync_all()
        .map_err(|error| from_io("executable could not be flushed", error))?;
    apply_mode(destination, 0o755)?;
    Ok(written)
}

pub mod build {
    //! A writer for both dialects: the reader's expectations, executable.
    //!
    //! Having both halves here lets a test prove the reader accepts a correct
    //! archive and refuses each specific corruption of it, rather than
    //! asserting against a fixture nobody can regenerate. It is public because
    //! the software lifecycle's own tests need to produce an artifact of
    //! exactly this shape, and building one elsewhere would be rebuilding the
    //! part most likely to drift.

    use super::BLOCK;

    /// Which dialect to emit.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Dialect {
        /// POSIX `ustar`, splitting long paths across `prefix` and `name`.
        Posix,
        /// GNU tar, carrying long paths in a preceding `L` entry.
        Gnu,
    }

    impl Dialect {
        const fn magic(self) -> &'static [u8; 8] {
            match self {
                // `ustar\0` then version `00`.
                Self::Posix => b"ustar\x0000",
                // GNU writes the magic and version as one space-padded field.
                Self::Gnu => b"ustar  \0",
            }
        }
    }

    /// One entry to write.
    pub struct Item<'a> {
        /// The path inside the archive.
        pub path: &'a str,
        /// The content. Empty for a directory.
        pub body: &'a [u8],
        /// The permission bits.
        pub mode: u32,
        /// Whether this is a directory.
        pub directory: bool,
    }

    impl<'a> Item<'a> {
        /// A regular file.
        #[must_use]
        pub const fn file(path: &'a str, body: &'a [u8], mode: u32) -> Self {
            Self {
                path,
                body,
                mode,
                directory: false,
            }
        }

        /// A directory.
        #[must_use]
        pub const fn directory(path: &'a str) -> Self {
            Self {
                path,
                body: &[],
                mode: 0o755,
                directory: true,
            }
        }
    }

    fn write_field(block: &mut [u8; BLOCK], at: usize, text: &[u8]) {
        block[at..at + text.len()].copy_from_slice(text);
    }

    fn header(path: &str, size: u64, mode: u32, flag: u8, dialect: Dialect) -> [u8; BLOCK] {
        let mut block = [0_u8; BLOCK];
        let (prefix, name) = match dialect {
            Dialect::Posix if path.len() > 100 => match path[..100].rfind('/') {
                Some(cut) => (&path[..cut], &path[cut + 1..]),
                None => ("", path),
            },
            _ => ("", path),
        };
        write_field(&mut block, 0, name.as_bytes());
        write_field(&mut block, 100, format!("{mode:07o}\0").as_bytes());
        write_field(&mut block, 108, b"0000000\0");
        write_field(&mut block, 116, b"0000000\0");
        write_field(&mut block, 124, format!("{size:011o}\0").as_bytes());
        write_field(&mut block, 136, b"00000000000\0");
        block[156] = flag;
        write_field(&mut block, 257, dialect.magic());
        if !prefix.is_empty() {
            write_field(&mut block, 345, prefix.as_bytes());
        }

        // The checksum is computed with its own field read as spaces.
        write_field(&mut block, 148, b"        ");
        let sum: u64 = block.iter().map(|byte| u64::from(*byte)).sum();
        write_field(&mut block, 148, format!("{sum:06o}\0 ").as_bytes());
        block
    }

    fn push_entry(out: &mut Vec<u8>, block: &[u8; BLOCK], body: &[u8]) {
        out.extend_from_slice(block);
        out.extend_from_slice(body);
        let remainder = body.len() % BLOCK;
        if remainder != 0 {
            out.extend(std::iter::repeat_n(0_u8, BLOCK - remainder));
        }
    }

    /// Write a tar stream in the given dialect.
    #[must_use]
    pub fn tar(items: &[Item<'_>], dialect: Dialect) -> Vec<u8> {
        let mut out = Vec::new();
        for item in items {
            let stored = if item.directory {
                format!("{}/", item.path.trim_end_matches('/'))
            } else {
                item.path.to_owned()
            };
            if dialect == Dialect::Gnu && stored.len() > 100 {
                let mut name = stored.clone().into_bytes();
                name.push(0);
                let long = header("././@LongLink", name.len() as u64, 0o644, b'L', dialect);
                push_entry(&mut out, &long, &name);
            }
            let flag = if item.directory { b'5' } else { b'0' };
            let block = header(&stored, item.body.len() as u64, item.mode, flag, dialect);
            push_entry(&mut out, &block, item.body);
        }
        // The end-of-archive marker: two zero blocks.
        out.extend(std::iter::repeat_n(0_u8, BLOCK * 2));
        out
    }

    /// Wrap bytes in a gzip member.
    #[must_use]
    pub fn gzip(payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
        out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(payload, 6));
        out.extend_from_slice(&crate::checksum::crc32(payload).to_le_bytes());
        #[allow(clippy::cast_possible_truncation)]
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out
    }

    /// A complete gzip-compressed tar, the shape every vendor ships.
    #[must_use]
    pub fn gzip_tar(items: &[Item<'_>], dialect: Dialect) -> Vec<u8> {
        gzip(&tar(items, dialect))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::path::PathBuf;

    use super::build::{Dialect, Item, gzip, gzip_tar, tar};
    use super::*;

    const ROOMY: Limits = Limits {
        entries: 4096,
        bytes: 1 << 30,
    };

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("setup-core-archive-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn read(root: &Path, relative: &str) -> Vec<u8> {
        fs::read(root.join(relative)).unwrap()
    }

    #[test]
    fn a_posix_archive_shaped_like_claudes_round_trips() {
        // Four flat files, one of them the executable. Measured from
        // `@anthropic-ai/claude-code-linux-x64`.
        let archive = gzip_tar(
            &[
                Item::file("package/claude", b"ELF...", 0o755),
                Item::file("package/package.json", b"{}", 0o644),
                Item::file("package/README.md", b"read me", 0o644),
                Item::file("package/LICENSE.md", b"licence", 0o644),
            ],
            Dialect::Posix,
        );
        let into = scratch("posix");
        let written = extract_gzip_tar(archive.as_slice(), &into, ROOMY).unwrap();

        assert_eq!(written.len(), 4);
        assert_eq!(read(&into, "package/claude"), b"ELF...");
        assert_eq!(read(&into, "package/package.json"), b"{}");
        assert!(written[0].is_executable());
        assert!(!written[1].is_executable());
        fs::remove_dir_all(&into).unwrap();
    }

    #[test]
    fn a_gnu_archive_with_long_names_and_directories_round_trips() {
        // Cursor's shape: GNU magic, real directories, and paths past the
        // 100-byte `name` field carried in `L` headers. Python's `tarfile`
        // hides this difference; the reader must not.
        let long = "dist-package/node_modules/better-sqlite3/build/Release/obj.target/deps/sqlite3/very/deeply/nested/better_sqlite3.node";
        assert!(
            long.len() > 100,
            "the fixture must exercise the long-name path"
        );
        let archive = gzip_tar(
            &[
                Item::directory("dist-package"),
                Item::directory("dist-package/node_modules"),
                Item::file("dist-package/cursor-agent", b"#!/bin/sh\n", 0o755),
                Item::file(long, b"native module", 0o755),
            ],
            Dialect::Gnu,
        );
        let into = scratch("gnu");
        let written = extract_gzip_tar(archive.as_slice(), &into, ROOMY).unwrap();

        assert_eq!(written.len(), 4);
        assert_eq!(written[0].kind, Kind::Directory);
        assert_eq!(written[3].path, long);
        assert_eq!(read(&into, long), b"native module");
        assert!(into.join("dist-package/node_modules").is_dir());
        fs::remove_dir_all(&into).unwrap();
    }

    #[test]
    fn a_single_file_archive_shaped_like_antigravitys_round_trips() {
        let archive = gzip_tar(
            &[Item::file("antigravity", b"one binary", 0o755)],
            Dialect::Gnu,
        );
        let into = scratch("single");
        let written = extract_gzip_tar(archive.as_slice(), &into, ROOMY).unwrap();
        assert_eq!(written.len(), 1);
        assert_eq!(read(&into, "antigravity"), b"one binary");
        fs::remove_dir_all(&into).unwrap();
    }

    /// Replace the type flag of the first header inside an uncompressed tar.
    fn with_type_flag(items: &[Item<'_>], flag: u8) -> Vec<u8> {
        let mut raw = tar(items, Dialect::Posix);
        raw[156] = flag;
        // The checksum covered the old flag, so recompute it or the reader
        // would refuse for the wrong reason and the test would prove nothing.
        for byte in &mut raw[148..156] {
            *byte = b' ';
        }
        let sum: u64 = raw[..BLOCK].iter().map(|byte| u64::from(*byte)).sum();
        raw[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        gzip(&raw)
    }

    #[test]
    fn every_entry_type_outside_file_and_directory_is_refused_by_name() {
        let items = [Item::file("payload", b"x", 0o644)];
        for (flag, expected) in [
            (b'1', "a hard link"),
            (b'2', "a symbolic link"),
            (b'3', "a character device"),
            (b'4', "a block device"),
            (b'6', "a FIFO"),
            (b'7', "a contiguous file"),
            (b'x', "a pax extended header"),
            (b'K', "a GNU long link name"),
        ] {
            let into = scratch(&format!("flag-{flag}"));
            let error = extract_gzip_tar(with_type_flag(&items, flag).as_slice(), &into, ROOMY)
                .unwrap_err();
            assert_eq!(error.reason(), ReasonCode::IntegrityMismatch);
            assert!(
                error.detail().contains(expected),
                "refusal for {:?} does not name it: {}",
                char::from(flag),
                error.detail()
            );
            let _ = fs::remove_dir_all(&into);
        }
    }

    #[test]
    fn a_path_that_climbs_out_of_the_destination_is_refused() {
        let archive = gzip_tar(&[Item::file("../escaped", b"x", 0o644)], Dialect::Posix);
        let into = scratch("climb");
        let error = extract_gzip_tar(archive.as_slice(), &into, ROOMY).unwrap_err();
        assert!(error.detail().contains("climbs out"), "{}", error.detail());
        let _ = fs::remove_dir_all(&into);
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let archive = gzip_tar(&[Item::file("/etc/passwd", b"x", 0o644)], Dialect::Posix);
        let into = scratch("absolute");
        let error = extract_gzip_tar(archive.as_slice(), &into, ROOMY).unwrap_err();
        assert!(
            error.detail().contains("absolute path"),
            "{}",
            error.detail()
        );
        let _ = fs::remove_dir_all(&into);
    }

    #[test]
    fn a_backslash_path_is_refused_because_it_names_a_different_file_per_system() {
        let archive = gzip_tar(&[Item::file("dir\\file", b"x", 0o644)], Dialect::Posix);
        let into = scratch("backslash");
        let error = extract_gzip_tar(archive.as_slice(), &into, ROOMY).unwrap_err();
        assert!(error.detail().contains("backslash"), "{}", error.detail());
        let _ = fs::remove_dir_all(&into);
    }

    #[test]
    fn bytes_that_are_not_a_gzip_member_are_refused_before_anything_is_written() {
        let into = scratch("notgzip");
        let error =
            extract_gzip_tar(b"PK\x03\x04 this is a zip".as_slice(), &into, ROOMY).unwrap_err();
        assert!(
            error.detail().contains("not a gzip member"),
            "{}",
            error.detail()
        );
        assert!(!into.exists() || fs::read_dir(&into).unwrap().next().is_none());
        let _ = fs::remove_dir_all(&into);
    }

    #[test]
    fn a_corrupted_payload_fails_the_gzip_checksum() {
        let mut archive = gzip_tar(
            &[Item::file("payload", b"honest bytes", 0o644)],
            Dialect::Posix,
        );
        // Break the stored CRC rather than the data: the DEFLATE stream stays
        // decodable, so the only thing that can catch this is the trailer.
        let at = archive.len() - 8;
        archive[at] ^= 0xFF;
        let into = scratch("crc");
        let error = extract_gzip_tar(archive.as_slice(), &into, ROOMY).unwrap_err();
        assert!(
            error.detail().contains("CRC-32 mismatch"),
            "{}",
            error.detail()
        );
        let _ = fs::remove_dir_all(&into);
    }

    #[test]
    fn a_truncated_archive_is_refused_rather_than_treated_as_complete() {
        let archive = gzip_tar(
            &[Item::file("payload", &[7_u8; 4096], 0o644)],
            Dialect::Posix,
        );
        let into = scratch("truncated");
        let cut = archive.len() / 2;
        let error = extract_gzip_tar(&archive[..cut], &into, ROOMY).unwrap_err();
        assert!(
            error.detail().contains("ended") || error.detail().contains("truncated"),
            "{}",
            error.detail()
        );
        let _ = fs::remove_dir_all(&into);
    }

    #[test]
    fn an_archive_past_the_entry_limit_is_stopped() {
        let bodies: Vec<String> = (0..8).map(|index| format!("file-{index}")).collect();
        let items: Vec<Item<'_>> = bodies
            .iter()
            .map(|name| Item::file(name, b"x", 0o644))
            .collect();
        let into = scratch("entries");
        let limits = Limits {
            entries: 3,
            bytes: 1 << 20,
        };
        let error = extract_gzip_tar(gzip_tar(&items, Dialect::Posix).as_slice(), &into, limits)
            .unwrap_err();
        assert!(
            error.detail().contains("more than the 3 entries"),
            "{}",
            error.detail()
        );
        let _ = fs::remove_dir_all(&into);
    }

    #[test]
    fn an_archive_that_inflates_past_the_byte_limit_is_stopped() {
        let archive = gzip_tar(
            &[Item::file("big", &vec![0_u8; 100_000], 0o644)],
            Dialect::Posix,
        );
        let into = scratch("bytes");
        let limits = Limits {
            entries: 16,
            bytes: 4096,
        };
        let error = extract_gzip_tar(archive.as_slice(), &into, limits).unwrap_err();
        assert!(
            error.detail().contains("inflates past"),
            "{}",
            error.detail()
        );
        let _ = fs::remove_dir_all(&into);
    }

    #[test]
    fn plain_bytes_are_placed_as_an_executable() {
        let into = scratch("raw");
        let destination = into.join("bin/grok");
        let written = place_executable(b"a whole program".as_slice(), &destination).unwrap();
        assert_eq!(written, 15);
        assert_eq!(fs::read(&destination).unwrap(), b"a whole program");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&destination).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755, "the placed artifact must be runnable");
        }
        fs::remove_dir_all(&into).unwrap();
    }

    #[test]
    fn a_header_whose_checksum_does_not_match_is_refused() {
        let mut raw = tar(&[Item::file("payload", b"x", 0o644)], Dialect::Posix);
        raw[0] = b'X';
        let into = scratch("checksum");
        let error = extract_gzip_tar(gzip(&raw).as_slice(), &into, ROOMY).unwrap_err();
        assert!(
            error.detail().contains("checksum mismatch"),
            "{}",
            error.detail()
        );
        let _ = fs::remove_dir_all(&into);
    }
}
