//! Content hashing for dirty detection and index invalidation (Task 1.7).
//!
//! xxHash3, not a cryptographic hash. The question being answered is "did
//! these bytes change", asked on every save, every watcher event, and every
//! index pass over a 500k-file tree. A collision costs a stale cache entry, so
//! the correct trade is the fastest hash with a negligible accidental
//! collision rate, and xxh3 runs at memory bandwidth where SHA-256 does not.
//!
//! Anywhere an *adversary* could choose the bytes (plugin signing, update
//! verification) uses a real cryptographic hash instead; those live in their
//! own tasks and deliberately do not come through here.
//!
//! Streaming matters as much as speed. [`hash_reader`] reads through a fixed
//! buffer, so hashing a 2GB file costs 64KB of memory rather than 2GB.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use xxhash_rust::xxh3::Xxh3;

/// Read buffer for streaming hashes. Large enough to keep syscall overhead
/// irrelevant, small enough to stay in cache.
const CHUNK_BYTES: usize = 64 * 1024;

/// A content hash, rendered as fixed-width lowercase hex so it can be
/// compared as a string on the frontend and used as a cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash(u64);

impl ContentHash {
    pub fn value(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Hash a byte slice already in memory.
pub fn hash_bytes(bytes: &[u8]) -> ContentHash {
    ContentHash(xxhash_rust::xxh3::xxh3_64(bytes))
}

/// Hash a reader's full contents without holding them in memory.
pub fn hash_reader<R: Read>(reader: R) -> io::Result<ContentHash> {
    let mut reader = BufReader::with_capacity(CHUNK_BYTES, reader);
    let mut hasher = Xxh3::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(ContentHash(hasher.digest()))
}

/// Hash a file on disk, streaming it.
pub fn hash_file(path: impl AsRef<Path>) -> io::Result<ContentHash> {
    hash_reader(File::open(path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_bytes_hash_identically() {
        assert_eq!(hash_bytes(b"hello world"), hash_bytes(b"hello world"));
    }

    #[test]
    fn a_one_byte_change_changes_the_hash() {
        assert_ne!(hash_bytes(b"hello world"), hash_bytes(b"hello worle"));
    }

    #[test]
    fn a_transposition_changes_the_hash() {
        // The failure mode a naive additive checksum has, and the reason dirty
        // detection cannot use one: `ab` and `ba` must not look identical.
        assert_ne!(hash_bytes(b"ab"), hash_bytes(b"ba"));
    }

    #[test]
    fn the_empty_input_has_a_stable_hash() {
        assert_eq!(hash_bytes(b""), hash_bytes(b""));
        assert_ne!(hash_bytes(b""), hash_bytes(b"\0"));
    }

    #[test]
    fn streaming_matches_the_in_memory_hash_across_chunk_boundaries() {
        // The bug this catches is a streaming hasher that resets or misaligns
        // per chunk, which only shows up past the buffer size.
        for size in [0usize, 1, CHUNK_BYTES - 1, CHUNK_BYTES, CHUNK_BYTES * 3 + 7] {
            let bytes: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let streamed = hash_reader(bytes.as_slice()).unwrap();
            assert_eq!(streamed, hash_bytes(&bytes), "size={size}");
        }
    }

    #[test]
    fn a_hash_renders_as_fixed_width_hex() {
        let rendered = hash_bytes(b"anything").to_string();
        assert_eq!(rendered.len(), 16);
        assert!(rendered.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
