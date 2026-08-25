//! CRC-32, the checksum both archive formats carry.
//!
//! This is not a digest and is kept away from [`crate::digest`] deliberately.
//! SHA-256 decides whether bytes are the ones a plan named; CRC-32 only detects
//! that a stream arrived intact, and the two must never be confused at a call
//! site. Nothing here is a security property.
//!
//! Written rather than taken as a dependency: it is twenty lines, ZIP and gzip
//! use the same polynomial, and a checksum that decides whether to trust a
//! stream is not a good place to add a supply-chain edge.

/// The reflected polynomial ZIP and gzip share.
const POLYNOMIAL: u32 = 0xEDB8_8320;

/// CRC-32 over a complete buffer.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    continue_crc32(0, bytes)
}

/// CRC-32 over one more piece of a stream.
///
/// Streaming matters here: the archives this checks inflate to hundreds of
/// megabytes, and buffering one to checksum it would defeat the point of
/// reading it incrementally.
#[must_use]
pub fn continue_crc32(seed: u32, bytes: &[u8]) -> u32 {
    let mut crc = !seed;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (POLYNOMIAL & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_check_value_is_reproduced() {
        // The value every CRC-32 implementation is checked against.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn the_empty_input_checksums_to_zero() {
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn streaming_in_pieces_agrees_with_hashing_the_whole() {
        let whole = b"the quick brown fox jumps over the lazy dog";
        let mut running = 0;
        for piece in whole.chunks(7) {
            running = continue_crc32(running, piece);
        }
        assert_eq!(running, crc32(whole));
    }
}
