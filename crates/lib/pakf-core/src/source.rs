//! The `source` module holds the helpers that read the files of the
//! repository: the hash of a file's bytes, the line ending of a file's first
//! line, and the read of a file that may be absent.

use std::{fs, path::Path};

use crate::{Error, Result};

/// `fnv1a64` returns the 64-bit FNV-1a hash of `bytes`. The tool uses one
/// hash, because FNV-1a gives the same value on every Rust version, so a
/// lock stays valid after the toolchain changes.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// `LineEnding` is the line ending of a file's first line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    /// A line feed (`\n`).
    Lf,
    /// A carriage return and line feed (`\r\n`).
    CrLf,
}

impl LineEnding {
    /// `detect_bytes` returns the line ending of the first line of `bytes`,
    /// or LF when `bytes` has no line break.
    pub fn detect_bytes(bytes: &[u8]) -> Self {
        match bytes.iter().position(|&b| b == b'\n') {
            Some(i) if i > 0 && bytes[i - 1] == b'\r' => LineEnding::CrLf,
            _ => LineEnding::Lf,
        }
    }
}

/// `read_optional` reads a file on disk as text, or returns `None` when the
/// file is absent.
pub(crate) fn read_optional(fs_path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(fs_path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(fs_path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_fnv1a() {
        assert_eq!(fnv1a64(b""), 0xCBF2_9CE4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xAF63_DC4C_8601_EC8C);
    }

    #[test]
    fn the_line_ending_is_the_first_lines() {
        assert_eq!(LineEnding::detect_bytes(b"a\r\nb\n"), LineEnding::CrLf);
        assert_eq!(LineEnding::detect_bytes(b"a\nb\r\n"), LineEnding::Lf);
        assert_eq!(LineEnding::detect_bytes(b"\n"), LineEnding::Lf);
        assert_eq!(LineEnding::detect_bytes(b"a"), LineEnding::Lf);
    }
}
