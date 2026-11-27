//! This module reads and writes the line endings of a pack's files in the
//! user's repository. A checkout of the repository can turn each LF of a
//! file into CRLF, so the pack reads each CRLF as LF, as git's `text=auto`
//! does on a commit, and writes a file over a CRLF copy with CRLF (the
//! decision "The committed bytes" of design-pack). It changes no line
//! ending of a binary file, and reads no CRLF of a file that holds a NUL
//! byte as LF, as git converts none of its line endings (the decision
//! "Binary files" of design-pack).

use std::borrow::Cow;

use super::lock::hash_of;
use crate::LineEnding;

/// `is_binary` returns whether `bytes` are a binary file: bytes that aren't
/// valid UTF-8 or that hold a NUL byte, as git reads a file with one.
// sokf-implements: CLI-1_AC-23
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
}

/// `normalise` returns `bytes` with each CRLF turned into LF. A carriage
/// return that no line feed follows stays, and bytes that hold a NUL byte
/// stay as they are, because git's `text=auto` reads them as binary and
/// converts none of their line endings, while it converts a file that
/// isn't UTF-8 and holds no NUL byte as text.
// sokf-implements: CLI-1_AC-21
// sokf-implements: CLI-1_AC-24
pub fn normalise(bytes: &[u8]) -> Cow<'_, [u8]> {
    if !has_crlf(bytes) || bytes.contains(&0) {
        return Cow::Borrowed(bytes);
    }
    let mut out = Vec::with_capacity(bytes.len());
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'\r' || bytes.get(i + 1) != Some(&b'\n') {
            out.push(b);
        }
    }
    Cow::Owned(out)
}

/// `same_text` returns whether `a` and `b` hold the same bytes once each CRLF
/// is read as LF, as `normalise` reads it. Two texts that it calls
/// different can still mean the same, as the module `meaning` reads them.
pub fn same_text(a: &[u8], b: &[u8]) -> bool {
    normalise(a) == normalise(b)
}

/// `is_base` returns whether `bytes` are the base whose hash is `base`: the
/// hash of the bytes, or of the text with each CRLF read as LF, is `base`.
/// The first hash matches a file that a pack committed with CRLF, and the
/// second a file without a NUL byte that a checkout converted to CRLF.
// sokf-implements: CLI-1_AC-22
pub fn is_base(bytes: &[u8], base: &str) -> bool {
    hash_of(bytes) == base || hash_of(&normalise(bytes)) == base
}

/// A `Reading` is how a file of the user's working tree holds the pack's
/// version of it: as its bytes, with the CRLF that a checkout wrote for
/// each LF, or with the LF that a commit of a text file wrote for each
/// CRLF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// The bytes are the version's.
    Bytes,
    /// Each CRLF of the bytes is an LF of the version.
    Lf,
    /// Each LF of the bytes is a CRLF of the version.
    Crlf,
}

impl Reading {
    /// `read` returns `bytes` read as the reading says, and bytes that hold
    /// a NUL byte as they are, as git converts none of their line endings.
    pub fn read(self, bytes: &[u8]) -> Cow<'_, [u8]> {
        match self {
            _ if bytes.contains(&0) => Cow::Borrowed(bytes),
            Reading::Bytes => Cow::Borrowed(bytes),
            Reading::Lf => normalise(bytes),
            Reading::Crlf => Cow::Owned(with_crlf(&normalise(bytes))),
        }
    }
}

/// `committed` returns the reading in which the base copy `bytes` holds the
/// pack's version whose hash is `base`, which the pack's repository holds,
/// or none where no reading hashes to `base`. A file with a NUL byte has
/// only its bytes, as git converts none of its line endings.
// sokf-implements: CLI-8_AC-13
pub fn committed(bytes: &[u8], base: &str) -> Option<Reading> {
    let readings: &[Reading] = if bytes.contains(&0) {
        &[Reading::Bytes]
    } else {
        &[Reading::Bytes, Reading::Lf, Reading::Crlf]
    };
    readings
        .iter()
        .copied()
        .find(|r| hash_of(&r.read(bytes)) == base)
}

/// `has_crlf` returns whether `bytes` hold a CRLF line ending.
pub fn has_crlf(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|w| w == b"\r\n")
}

/// `with_crlf` returns `lf`, whose lines end with LF, with each line ending
/// written as CRLF.
fn with_crlf(lf: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(lf.len() + lf.len() / 32);
    for &b in lf {
        if b == b'\n' {
            out.push(b'\r');
        }
        out.push(b);
    }
    out
}

/// `keep_ending` returns `text`, which sync writes over the copy `existing`,
/// with the line ending of the copy's first line, as `validate --fix` keeps
/// a file's line ending: each line ends with CRLF when the copy's first line
/// does, and `text` is unchanged otherwise, or where either is binary.
// sokf-implements: CLI-2_AC-21
// sokf-implements: CLI-1_AC-24
pub fn keep_ending(existing: &[u8], text: Vec<u8>) -> Vec<u8> {
    if LineEnding::detect_bytes(existing) == LineEnding::Lf
        || is_binary(existing)
        || is_binary(&text)
    {
        return text;
    }
    with_crlf(&normalise(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_crlf_turns_into_lf() {
        assert_eq!(&*normalise(b"a\r\nb\rc\r\r\n\r"), b"a\nb\rc\r\n\r");
        assert!(matches!(normalise(b"a\nb\r"), Cow::Borrowed(_)));
        assert!(same_text(b"a\r\nb\n", b"a\nb\r\n"));
        assert!(!same_text(b"a\rb\n", b"a\nb\n"));
    }

    #[test]
    fn a_file_is_its_base_with_either_line_ending() {
        let lf = hash_of(b"a\nb\n");
        assert!(is_base(b"a\nb\n", &lf));
        assert!(is_base(b"a\r\nb\r\n", &lf));
        assert!(!is_base(b"a\nc\n", &lf));
        let crlf = hash_of(b"a\r\nb\r\n");
        assert!(is_base(b"a\r\nb\r\n", &crlf));
    }

    #[test]
    fn a_file_that_isnt_utf8_or_holds_a_nul_byte_is_binary() {
        assert!(is_binary(b"MIT \xff\n"));
        assert!(is_binary(b"a\x00b\n"));
        assert!(!is_binary(b"caf\xc3\xa9\r\n"));
        assert!(!is_binary(b""));
    }

    // sokf-tests: CLI-1_AC-24
    #[test]
    fn no_line_ending_of_a_binary_file_changes() {
        assert_eq!(&*normalise(b"a\r\n\x00"), b"a\r\n\x00");
        assert!(!same_text(b"a\r\n\x00", b"a\n\x00"));
        assert!(!is_base(b"a\r\n\x00", &hash_of(b"a\n\x00")));
        // A file that isn't UTF-8 and holds no NUL byte is one that git's
        // checkout converts, so its CRLF reads as LF, but a write over it
        // changes none of its line endings.
        assert_eq!(&*normalise(b"a\r\n\xff"), b"a\n\xff");
        assert!(is_base(b"a\r\n\xff", &hash_of(b"a\n\xff")));
        assert_eq!(keep_ending(b"x\r\n", b"a\n\x00".to_vec()), b"a\n\x00");
        assert_eq!(keep_ending(b"x\r\n\xff", b"a\n".to_vec()), b"a\n");
    }

    #[test]
    fn the_committed_version_is_the_one_whose_hash_the_lock_records() {
        let crlf = b"a\r\nb\r\n";
        assert_eq!(committed(crlf, &hash_of(crlf)), Some(Reading::Bytes));
        assert_eq!(committed(b"a\nb\n", &hash_of(crlf)), Some(Reading::Crlf));
        assert_eq!(committed(crlf, &hash_of(b"a\nb\n")), Some(Reading::Lf));
        assert_eq!(committed(b"a\n", &hash_of(b"b\n")), None);
        assert_eq!(&*Reading::Crlf.read(b"a\nb\r\n"), crlf);
        assert_eq!(&*Reading::Crlf.read(b"a\r\n\x00b\n"), b"a\r\n\x00b\n");
    }

    #[test]
    fn a_write_keeps_the_line_ending_of_the_copy() {
        assert_eq!(
            keep_ending(b"x\r\ny\n", b"a\nb\r\nc".to_vec()),
            b"a\r\nb\r\nc"
        );
        assert_eq!(keep_ending(b"x\ny\r\n", b"a\nb\r\n".to_vec()), b"a\nb\r\n");
        assert_eq!(keep_ending(b"", b"a\n".to_vec()), b"a\n");
    }
}
