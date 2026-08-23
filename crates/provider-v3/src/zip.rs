//! A reader for the one ZIP shape `ai-stp-bundle/1` is allowed to be.
//!
//! The bundle format is a *canonical* ZIP: stored members only, members in a
//! fixed order, every timestamp pinned to 1980-01-01T00:00:00, Unix creator,
//! and empty extra and comment fields. Two builds of the same input must produce
//! the same bytes, which is what makes `artifact_digest` mean anything.
//!
//! That is why this reader exists instead of a general-purpose one. A library
//! that accepts every legal ZIP would happily read a bundle whose members were
//! reordered, compressed, or stamped with a local clock — and the provider would
//! then install something whose bytes nobody can reproduce. Here the metadata is
//! *checked*, not tolerated, and anything outside the canonical shape is a
//! refusal.
//!
//! Only what the format uses is implemented: no compression, no encryption, no
//! ZIP64, no multi-disk. Each of those is refused by name rather than ignored.

use crate::error::{Error, Result};
use crate::reason::WireReason;

/// The DOS date for 1980-01-01, the only date this format admits.
const CANONICAL_DOS_DATE: u16 = 0x0021;

/// The DOS time for 00:00:00.
const CANONICAL_DOS_TIME: u16 = 0x0000;

/// The `version made by` high byte for a Unix creator.
const UNIX_CREATOR: u8 = 3;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const END_OF_CENTRAL_DIRECTORY: u32 = 0x0605_4b50;

/// One member of a canonical bundle ZIP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The member name, exactly as stored.
    pub name: String,
    /// The member's bytes.
    pub data: Vec<u8>,
    /// The Unix mode the archive records, when it records one.
    pub mode: Option<u32>,
}

/// Read every member of a canonical ZIP, in stored order.
///
/// # Errors
///
/// Refuses anything outside the canonical shape: compression, a timestamp other
/// than the pinned one, a non-Unix creator, extra or comment bytes, a CRC that
/// does not match the data, ZIP64, or a multi-disk archive.
pub fn read(bytes: &[u8]) -> Result<Vec<Member>> {
    let eocd = find_end_of_central_directory(bytes)?;
    let disk = read_u16(bytes, eocd + 4)?;
    let directory_disk = read_u16(bytes, eocd + 6)?;
    if disk != 0 || directory_disk != 0 {
        return Err(refuse("a bundle is a single-disk archive"));
    }
    let entry_count = read_u16(bytes, eocd + 10)? as usize;
    if read_u16(bytes, eocd + 8)? as usize != entry_count {
        return Err(refuse("the central directory spans disks"));
    }
    if read_u16(bytes, eocd + 20)? != 0 {
        return Err(refuse("a bundle carries no archive comment"));
    }
    let directory_offset = read_u32(bytes, eocd + 16)? as usize;

    let mut members = Vec::with_capacity(entry_count);
    let mut cursor = directory_offset;
    for index in 0..entry_count {
        if read_u32(bytes, cursor)? != CENTRAL_HEADER {
            return Err(refuse(format!(
                "central directory entry {index} is malformed"
            )));
        }
        let creator = read_u16(bytes, cursor + 4)?;
        if u8::try_from(creator >> 8).unwrap_or(0) != UNIX_CREATOR {
            return Err(refuse("every bundle member is written by a Unix creator"));
        }
        let flags = read_u16(bytes, cursor + 8)?;
        if flags != 0 {
            return Err(refuse("a bundle member sets no general-purpose flags"));
        }
        let method = read_u16(bytes, cursor + 10)?;
        if method != 0 {
            return Err(refuse("a bundle member is stored, never compressed"));
        }
        check_timestamp(read_u16(bytes, cursor + 12)?, read_u16(bytes, cursor + 14)?)?;
        let crc = read_u32(bytes, cursor + 16)?;
        let compressed = read_u32(bytes, cursor + 20)? as usize;
        let uncompressed = read_u32(bytes, cursor + 24)? as usize;
        if compressed != uncompressed {
            return Err(refuse("a stored member's two sizes disagree"));
        }
        let name_length = read_u16(bytes, cursor + 28)? as usize;
        let extra_length = read_u16(bytes, cursor + 30)? as usize;
        let comment_length = read_u16(bytes, cursor + 32)? as usize;
        if extra_length != 0 || comment_length != 0 {
            return Err(refuse("a bundle member carries no extra or comment field"));
        }
        let external_attributes = read_u32(bytes, cursor + 38)?;
        let local_offset = read_u32(bytes, cursor + 42)? as usize;
        let name = read_name(bytes, cursor + 46, name_length)?;

        let data = read_local_member(bytes, local_offset, &name, uncompressed)?;
        if crc32(&data) != crc {
            return Err(Error::refuse(
                WireReason::DigestMismatch,
                format!("member {name:?} does not match its recorded CRC"),
            ));
        }
        let mode = (external_attributes >> 16) & 0o7777;
        members.push(Member {
            name,
            data,
            mode: (mode != 0).then_some(mode),
        });
        cursor = cursor + 46 + name_length;
    }
    Ok(members)
}

fn read_local_member(bytes: &[u8], offset: usize, name: &str, length: usize) -> Result<Vec<u8>> {
    if read_u32(bytes, offset)? != LOCAL_HEADER {
        return Err(refuse(format!("member {name:?} has no local header")));
    }
    if read_u16(bytes, offset + 8)? != 0 {
        return Err(refuse(format!("member {name:?} is compressed")));
    }
    check_timestamp(read_u16(bytes, offset + 10)?, read_u16(bytes, offset + 12)?)?;
    let name_length = read_u16(bytes, offset + 26)? as usize;
    let extra_length = read_u16(bytes, offset + 28)? as usize;
    if extra_length != 0 {
        return Err(refuse(format!(
            "member {name:?} carries a local extra field"
        )));
    }
    if read_name(bytes, offset + 30, name_length)? != name {
        return Err(refuse(format!(
            "member {name:?} is named differently in its local header"
        )));
    }
    let start = offset + 30 + name_length;
    bytes
        .get(start..start.saturating_add(length))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| refuse(format!("member {name:?} is truncated")))
}

fn check_timestamp(time: u16, date: u16) -> Result<()> {
    if time != CANONICAL_DOS_TIME || date != CANONICAL_DOS_DATE {
        return Err(refuse(
            "a bundle member's timestamp is pinned to 1980-01-01T00:00:00; a local \
             clock in the archive would make the same input produce different bytes",
        ));
    }
    Ok(())
}

fn find_end_of_central_directory(bytes: &[u8]) -> Result<usize> {
    // The record is 22 bytes and must be last: the format allows no comment, so
    // there is nothing after it to search past.
    if bytes.len() < 22 {
        return Err(refuse("the artifact is too short to be a ZIP"));
    }
    let offset = bytes.len() - 22;
    if read_u32(bytes, offset)? != END_OF_CENTRAL_DIRECTORY {
        return Err(refuse(
            "the artifact does not end with a comment-free end-of-central-directory record",
        ));
    }
    Ok(offset)
}

fn read_name(bytes: &[u8], offset: usize, length: usize) -> Result<String> {
    let raw = bytes
        .get(offset..offset.saturating_add(length))
        .ok_or_else(|| refuse("a member name runs past the end of the artifact"))?;
    String::from_utf8(raw.to_vec()).map_err(|_| refuse("a member name is not valid UTF-8"))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let slice = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or_else(|| refuse("the artifact is truncated"))?;
    Ok(u16::from_le_bytes([
        *slice.first().unwrap_or(&0),
        *slice.get(1).unwrap_or(&0),
    ]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or_else(|| refuse("the artifact is truncated"))?;
    Ok(u32::from_le_bytes([
        *slice.first().unwrap_or(&0),
        *slice.get(1).unwrap_or(&0),
        *slice.get(2).unwrap_or(&0),
        *slice.get(3).unwrap_or(&0),
    ]))
}

fn refuse(detail: impl Into<String>) -> Error {
    Error::refuse(WireReason::UnsupportedBundleFormat, detail)
}

/// CRC-32, the variant ZIP uses.
///
/// Written here rather than taken as a dependency: it is twenty lines, and a
/// checksum this reader uses to decide whether to trust bytes is not a good
/// place to add a supply-chain edge.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

pub mod build {
    //! A canonical writer: the reader's statement of what it expects, executable.
    //!
    //! Having both halves here means a test can prove the reader accepts a
    //! correct archive and refuses each specific corruption of it, rather than
    //! asserting against a fixture nobody can regenerate. It is public because
    //! anything that needs to exercise a provider — a conformance run, a
    //! downstream test — needs to produce this exact shape, and reimplementing it
    //! elsewhere would be reimplementing the part most likely to drift.

    use super::{CANONICAL_DOS_DATE, CANONICAL_DOS_TIME, UNIX_CREATOR, crc32};

    /// One member to write.
    pub struct Entry {
        /// The member name.
        pub name: String,
        /// The member's bytes.
        pub data: Vec<u8>,
        /// The Unix mode to record.
        pub mode: u32,
    }

    /// Write a canonical stored ZIP.
    #[must_use]
    pub fn write(entries: &[Entry]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut directory = Vec::new();
        for entry in entries {
            let offset = u32::try_from(out.len()).unwrap_or(0);
            let crc = crc32(&entry.data);
            let size = u32::try_from(entry.data.len()).unwrap_or(0);
            let name = entry.name.as_bytes();

            out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
            out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0_u16.to_le_bytes()); // flags
            out.extend_from_slice(&0_u16.to_le_bytes()); // stored
            out.extend_from_slice(&CANONICAL_DOS_TIME.to_le_bytes());
            out.extend_from_slice(&CANONICAL_DOS_DATE.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&u16::try_from(name.len()).unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&0_u16.to_le_bytes()); // extra
            out.extend_from_slice(name);
            out.extend_from_slice(&entry.data);

            directory.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            directory
                .extend_from_slice(&((u16::from(UNIX_CREATOR) << 8) | 0x0014_u16).to_le_bytes());
            directory.extend_from_slice(&20_u16.to_le_bytes());
            directory.extend_from_slice(&0_u16.to_le_bytes());
            directory.extend_from_slice(&0_u16.to_le_bytes());
            directory.extend_from_slice(&CANONICAL_DOS_TIME.to_le_bytes());
            directory.extend_from_slice(&CANONICAL_DOS_DATE.to_le_bytes());
            directory.extend_from_slice(&crc.to_le_bytes());
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&size.to_le_bytes());
            directory.extend_from_slice(&u16::try_from(name.len()).unwrap_or(0).to_le_bytes());
            directory.extend_from_slice(&0_u16.to_le_bytes()); // extra
            directory.extend_from_slice(&0_u16.to_le_bytes()); // comment
            directory.extend_from_slice(&0_u16.to_le_bytes()); // disk
            directory.extend_from_slice(&0_u16.to_le_bytes()); // internal attrs
            directory.extend_from_slice(&(entry.mode << 16).to_le_bytes());
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name);
        }

        let directory_offset = u32::try_from(out.len()).unwrap_or(0);
        let directory_size = u32::try_from(directory.len()).unwrap_or(0);
        let count = u16::try_from(entries.len()).unwrap_or(0);
        out.extend_from_slice(&directory);
        out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&directory_size.to_le_bytes());
        out.extend_from_slice(&directory_offset.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out
    }

    /// A 0o644 member from a string.
    #[must_use]
    pub fn entry(name: &str, data: &str) -> Entry {
        Entry {
            name: name.to_owned(),
            data: data.as_bytes().to_vec(),
            mode: 0o644,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use build::{Entry, entry, write};

    #[test]
    fn the_known_crc_of_a_known_string_is_produced() {
        // The standard check value for CRC-32/ISO-HDLC.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_canonical_archive_round_trips_in_stored_order() {
        let bytes = write(&[entry("bundle.json", "{}"), entry("files/a.md", "hello")]);
        let members = read(&bytes).unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].name, "bundle.json");
        assert_eq!(members[1].name, "files/a.md");
        assert_eq!(members[1].data, b"hello");
        assert_eq!(members[1].mode, Some(0o644));
    }

    #[test]
    fn an_executable_mode_survives_because_some_harness_surfaces_are_executable() {
        let bytes = write(&[Entry {
            name: "files/run.sh".to_owned(),
            data: b"#!/bin/sh\n".to_vec(),
            mode: 0o755,
        }]);
        assert_eq!(read(&bytes).unwrap()[0].mode, Some(0o755));
    }

    #[test]
    fn an_empty_archive_is_read_rather_than_refused() {
        assert!(read(&write(&[])).unwrap().is_empty());
    }

    /// Corrupt one little-endian field in the central directory of a one-entry
    /// archive. The directory begins right after the single local member.
    fn corrupt_central(bytes: &mut [u8], field_offset: usize, value: u16) {
        let eocd = bytes.len() - 22;
        let directory = u32::from_le_bytes([
            bytes[eocd + 16],
            bytes[eocd + 17],
            bytes[eocd + 18],
            bytes[eocd + 19],
        ]) as usize;
        let at = directory + field_offset;
        bytes[at] = value.to_le_bytes()[0];
        bytes[at + 1] = value.to_le_bytes()[1];
    }

    #[test]
    fn a_compressed_member_is_refused_rather_than_decompressed() {
        let mut bytes = write(&[entry("files/a.md", "hello")]);
        corrupt_central(&mut bytes, 10, 8); // deflate
        let error = read(&bytes).unwrap_err();
        assert!(
            error.detail().contains("stored, never compressed"),
            "{error}"
        );
    }

    #[test]
    fn a_local_clock_in_the_archive_is_refused() {
        // Two builds of the same input must produce the same bytes. A real
        // timestamp is exactly what breaks that.
        let mut bytes = write(&[entry("files/a.md", "hello")]);
        corrupt_central(&mut bytes, 14, 0x5A21);
        let error = read(&bytes).unwrap_err();
        assert!(error.detail().contains("pinned to 1980"), "{error}");
    }

    #[test]
    fn a_non_unix_creator_is_refused() {
        let mut bytes = write(&[entry("files/a.md", "hello")]);
        corrupt_central(&mut bytes, 4, 20); // creator 0 = MS-DOS
        assert!(read(&bytes).unwrap_err().detail().contains("Unix creator"));
    }

    #[test]
    fn a_general_purpose_flag_is_refused() {
        let mut bytes = write(&[entry("files/a.md", "hello")]);
        corrupt_central(&mut bytes, 8, 1); // encrypted
        assert!(
            read(&bytes)
                .unwrap_err()
                .detail()
                .contains("no general-purpose flags")
        );
    }

    #[test]
    fn an_extra_field_is_refused() {
        let mut bytes = write(&[entry("files/a.md", "hello")]);
        corrupt_central(&mut bytes, 30, 4);
        assert!(
            read(&bytes)
                .unwrap_err()
                .detail()
                .contains("no extra or comment")
        );
    }

    #[test]
    fn data_that_does_not_match_its_crc_is_a_digest_mismatch() {
        let mut bytes = write(&[entry("files/a.md", "hello")]);
        // The member data sits after the local header and the name.
        let position = bytes.windows(5).position(|w| w == b"hello").unwrap();
        bytes[position] = b'j';
        let error = read(&bytes).unwrap_err();
        assert_eq!(error.reason(), Some(WireReason::DigestMismatch));
    }

    #[test]
    fn an_artifact_with_a_trailing_comment_is_refused() {
        let mut bytes = write(&[entry("files/a.md", "hello")]);
        bytes.extend_from_slice(b"trailing");
        assert!(
            read(&bytes)
                .unwrap_err()
                .detail()
                .contains("end-of-central-directory")
        );
    }

    #[test]
    fn a_truncated_artifact_is_refused_rather_than_read_partially() {
        let bytes = write(&[entry("files/a.md", "hello")]);
        for cut in [0, 10, bytes.len() / 2, bytes.len() - 1] {
            assert!(
                read(bytes.get(..cut).unwrap_or(&[])).is_err(),
                "accepted a {cut}-byte prefix"
            );
        }
    }

    #[test]
    fn every_refusal_of_a_malformed_shape_names_the_format_not_the_content() {
        // A caller that sent a well-formed bundle should never see these; a
        // caller that sent something else should learn it is the format.
        let mut bytes = write(&[entry("files/a.md", "hello")]);
        corrupt_central(&mut bytes, 10, 8);
        assert_eq!(
            read(&bytes).unwrap_err().reason(),
            Some(WireReason::UnsupportedBundleFormat)
        );
    }
}
