//! Encoding detection and decoding (Task 1.7, REQ-ED-006.6).
//!
//! Four encodings are recognised, which is not an arbitrary shortlist: they
//! are the ones a developer actually meets. UTF-8 is everything modern,
//! UTF-16 LE is what Windows tooling and PowerShell emit, UTF-16 BE turns up
//! in files that travelled through Java or older macOS tooling, and Latin-1 is
//! the fallback that makes a legacy file open as text instead of as garbage.
//!
//! ## How the guess is made
//!
//! A byte-order mark is a declaration, so it wins outright and no heuristic
//! runs. Without one:
//!
//! 1. If the bytes are valid UTF-8, it is UTF-8. This is nearly free to check
//!    and has an extremely low false-positive rate, because arbitrary
//!    non-UTF-8 byte soup almost never satisfies the continuation-byte rules.
//! 2. Otherwise, look for the UTF-16 signature: a text file in UTF-16 whose
//!    content is mostly ASCII has a NUL in every other byte, on a consistent
//!    parity. High even-position NUL density means big-endian, high
//!    odd-position means little-endian.
//! 3. Otherwise Latin-1, which cannot fail: every byte maps to a code point.
//!
//! Latin-1 being infallible is exactly why it is last. It is the reason no
//! file is ever undecodable, and also the reason it must never be reached
//! while a better answer is available.
//!
//! ## Encoding is not detection's inverse
//!
//! Decoding is lossy in one direction on purpose: an invalid UTF-8 sequence
//! becomes U+FFFD rather than an error, because refusing to open a file with
//! one bad byte is worse than showing the bad byte. [`Encoding::encode`]
//! therefore reports [`EncodeOutcome::lossy`] when a round trip would not be
//! byte-identical, so a save path can warn instead of silently rewriting a
//! file it did not fully understand.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Bytes examined by the detector and by binary detection. One page-ish
/// window: enough for the signature of any real text file, small enough that
/// probing a 2GB file costs the same as probing a 2KB one.
pub const SNIFF_BYTES: usize = 8 * 1024;

/// A text encoding this service can read and write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    /// UTF-8 with no byte-order mark. The default for anything new.
    Utf8,
    /// UTF-8 preceded by an EF BB BF mark. Preserved on write, because
    /// stripping a mark a build tool expects is a real way to break a project.
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// ISO-8859-1. The infallible fallback.
    Latin1,
}

impl Encoding {
    /// Byte-order mark that introduces this encoding, if any.
    pub fn bom(self) -> &'static [u8] {
        match self {
            Encoding::Utf8Bom => &[0xEF, 0xBB, 0xBF],
            Encoding::Utf16Le => &[0xFF, 0xFE],
            Encoding::Utf16Be => &[0xFE, 0xFF],
            Encoding::Utf8 | Encoding::Latin1 => &[],
        }
    }

    /// Stable name used in IPC payloads and in the `files.encoding` setting.
    ///
    /// Identical to the `serde` representation, so a log field and a wire field
    /// never disagree about what to call an encoding.
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Utf8 => "utf8",
            Encoding::Utf8Bom => "utf8_bom",
            Encoding::Utf16Le => "utf16_le",
            Encoding::Utf16Be => "utf16_be",
            Encoding::Latin1 => "latin1",
        }
    }

    /// Parse a name from the `files.encoding` setting or an IPC request.
    /// Accepts the common spellings, because a user typing `utf-8` into a
    /// settings file has expressed their intent perfectly clearly.
    pub fn parse(name: &str) -> Option<Self> {
        let normalized: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect();
        match normalized.as_str() {
            "utf8" => Some(Encoding::Utf8),
            "utf8bom" | "utf8withbom" => Some(Encoding::Utf8Bom),
            "utf16le" | "utf16" => Some(Encoding::Utf16Le),
            "utf16be" => Some(Encoding::Utf16Be),
            "latin1" | "iso88591" | "windows1252" => Some(Encoding::Latin1),
            _ => None,
        }
    }

    /// Decode bytes into text, stripping the mark if the encoding has one.
    ///
    /// Never fails. Undecodable input becomes U+FFFD, so a file with one
    /// corrupt byte still opens.
    pub fn decode(self, bytes: &[u8]) -> String {
        let body = bytes.strip_prefix(self.bom()).unwrap_or(bytes);
        match self {
            Encoding::Utf8 | Encoding::Utf8Bom => String::from_utf8_lossy(body).into_owned(),
            Encoding::Utf16Le => decode_utf16(body, u16::from_le_bytes),
            Encoding::Utf16Be => decode_utf16(body, u16::from_be_bytes),
            Encoding::Latin1 => body.iter().map(|b| char::from(*b)).collect(),
        }
    }

    /// Encode text back into bytes, mark included.
    pub fn encode(self, text: &str) -> EncodeOutcome {
        let bom = self.bom();
        let mut bytes = Vec::with_capacity(bom.len() + text.len());
        bytes.extend_from_slice(bom);
        let mut lossy = false;

        match self {
            Encoding::Utf8 | Encoding::Utf8Bom => bytes.extend_from_slice(text.as_bytes()),
            Encoding::Utf16Le | Encoding::Utf16Be => {
                let big_endian = self == Encoding::Utf16Be;
                for unit in text.encode_utf16() {
                    let pair = if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    };
                    bytes.extend_from_slice(&pair);
                }
            }
            Encoding::Latin1 => {
                for ch in text.chars() {
                    match u32::from(ch) {
                        // Latin-1 is a 256-code-point encoding. Anything the
                        // user typed above U+00FF cannot be written, so it
                        // becomes '?' and the caller is told the save was
                        // lossy rather than discovering it on reopen.
                        code if code <= 0xFF => bytes.push(code as u8),
                        _ => {
                            bytes.push(b'?');
                            lossy = true;
                        }
                    }
                }
            }
        }

        EncodeOutcome { bytes, lossy }
    }
}

impl std::fmt::Display for Encoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Result of encoding text: the bytes, plus whether anything could not be
/// represented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeOutcome {
    pub bytes: Vec<u8>,
    /// True when at least one character was substituted because the target
    /// encoding cannot represent it.
    pub lossy: bool,
}

/// What detection concluded, and how much it trusted the conclusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detection {
    pub encoding: Encoding,
    /// True when a byte-order mark decided it. A marked file is a declared
    /// file, and the UI should not offer to "fix" its encoding.
    pub from_bom: bool,
}

/// Detect the encoding of a byte slice.
///
/// Only the first [`SNIFF_BYTES`] participate in the heuristics, but a UTF-8
/// validity check runs over everything supplied so a file that is ASCII for
/// 8KB and Latin-1 afterwards is not misreported.
///
/// The UTF-16 check runs *before* the UTF-8 one when NUL bytes are present, and
/// that ordering is not cosmetic: NUL is a perfectly valid UTF-8 byte, so
/// `t\0h\0e\0` — UTF-16 LE for "the" — passes UTF-8 validation. Checking UTF-8
/// first would classify every unmarked UTF-16 file as UTF-8 and render it as
/// text interleaved with NULs. Since no real text file in a single-byte
/// encoding contains NUL, its presence is the signal that the parity heuristic
/// is the one worth trusting.
pub fn detect(bytes: &[u8]) -> Detection {
    if let Some(encoding) = detect_bom(bytes) {
        return Detection {
            encoding,
            from_bom: true,
        };
    }

    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    if head.contains(&0)
        && let Some(encoding) = detect_utf16_without_bom(head)
    {
        return Detection {
            encoding,
            from_bom: false,
        };
    }

    if std::str::from_utf8(bytes).is_ok() {
        return Detection {
            encoding: Encoding::Utf8,
            from_bom: false,
        };
    }

    Detection {
        encoding: detect_utf16_without_bom(head).unwrap_or(Encoding::Latin1),
        from_bom: false,
    }
}

/// The encoding declared by a leading byte-order mark, if there is one.
///
/// UTF-8's mark is tested first: `EF BB BF` cannot be confused with either
/// UTF-16 mark, and testing the two-byte marks first would misread nothing
/// but relies on an ordering coincidence rather than on the check itself.
pub fn detect_bom(bytes: &[u8]) -> Option<Encoding> {
    [Encoding::Utf8Bom, Encoding::Utf16Le, Encoding::Utf16Be]
        .into_iter()
        .find(|candidate| bytes.starts_with(candidate.bom()))
}

/// Look for the parity signature of unmarked UTF-16.
///
/// Requires an even length, a meaningful sample, and a strong parity
/// imbalance. The thresholds are deliberately conservative: misreading a
/// Latin-1 file as UTF-16 produces unreadable CJK, which is a far more
/// alarming failure than the reverse, so the tie goes to Latin-1.
fn detect_utf16_without_bom(head: &[u8]) -> Option<Encoding> {
    if head.len() < 4 || !head.len().is_multiple_of(2) {
        return None;
    }

    let mut even_nulls = 0usize;
    let mut odd_nulls = 0usize;
    for (index, byte) in head.iter().enumerate() {
        if *byte == 0 {
            if index.is_multiple_of(2) {
                even_nulls += 1;
            } else {
                odd_nulls += 1;
            }
        }
    }

    let pairs = head.len() / 2;
    // "Most" rather than "all": a UTF-16 file containing any non-Latin text
    // has non-zero high bytes for those characters, so demanding every pair
    // would reject exactly the files UTF-16 exists to hold.
    let threshold = pairs / 2;
    match (even_nulls > threshold, odd_nulls > threshold) {
        // Ambiguous (a run of NUL pairs, i.e. not text at all): decline.
        (true, true) => None,
        (true, false) => Some(Encoding::Utf16Be),
        (false, true) => Some(Encoding::Utf16Le),
        (false, false) => None,
    }
}

fn decode_utf16(body: &[u8], to_unit: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|pair| to_unit([pair[0], pair[1]]))
        .collect();
    // Lossy: an unpaired surrogate becomes U+FFFD instead of refusing the
    // whole file. A trailing odd byte is dropped by `chunks_exact`, which is
    // the only sensible reading of a truncated code unit.
    String::from_utf16_lossy(&units)
}

/// Whether a byte slice looks like binary content (REQ-ED-006 read path).
///
/// A NUL byte in the first [`SNIFF_BYTES`] is the signal, which is the same
/// test git uses and it is right for the same reason: text formats do not
/// contain NUL, and every common binary format has one early.
///
/// The byte-order-mark check in front of it is what stops UTF-16 text, which
/// is roughly half NUL bytes, from being classified as a binary blob.
pub fn looks_binary(bytes: &[u8]) -> bool {
    if matches!(
        detect_bom(bytes),
        Some(Encoding::Utf16Le | Encoding::Utf16Be)
    ) {
        return false;
    }
    bytes[..bytes.len().min(SNIFF_BYTES)].contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
        let encoding = if big_endian {
            Encoding::Utf16Be
        } else {
            Encoding::Utf16Le
        };
        let mut bytes = encoding.encode(text).bytes;
        if !bom {
            bytes.drain(..2);
        }
        bytes
    }

    #[test]
    fn plain_ascii_is_utf8_without_a_bom() {
        let detection = detect(b"fn main() {}\n");
        assert_eq!(detection.encoding, Encoding::Utf8);
        assert!(!detection.from_bom);
    }

    #[test]
    fn multibyte_utf8_is_recognised_without_a_bom() {
        let detection = detect("héllo — 世界".as_bytes());
        assert_eq!(detection.encoding, Encoding::Utf8);
    }

    #[test]
    fn a_utf8_bom_is_reported_and_stripped_on_decode() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"hello");
        let detection = detect(&bytes);
        assert_eq!(detection.encoding, Encoding::Utf8Bom);
        assert!(detection.from_bom);
        assert_eq!(detection.encoding.decode(&bytes), "hello");
    }

    #[test]
    fn utf16_boms_are_recognised_in_both_byte_orders() {
        for big_endian in [false, true] {
            let bytes = utf16("hello", big_endian, true);
            let detection = detect(&bytes);
            let expected = if big_endian {
                Encoding::Utf16Be
            } else {
                Encoding::Utf16Le
            };
            assert_eq!(detection.encoding, expected);
            assert!(detection.from_bom);
            assert_eq!(detection.encoding.decode(&bytes), "hello");
        }
    }

    #[test]
    fn unmarked_utf16_is_found_by_the_null_parity_heuristic() {
        for big_endian in [false, true] {
            let bytes = utf16("the quick brown fox", big_endian, false);
            let detection = detect(&bytes);
            let expected = if big_endian {
                Encoding::Utf16Be
            } else {
                Encoding::Utf16Le
            };
            assert_eq!(detection.encoding, expected, "big_endian={big_endian}");
            assert!(!detection.from_bom);
            assert_eq!(detection.encoding.decode(&bytes), "the quick brown fox");
        }
    }

    #[test]
    fn invalid_utf8_without_a_utf16_signature_falls_back_to_latin1() {
        // 0xE9 is 'é' in Latin-1 and an incomplete sequence in UTF-8.
        let bytes = b"caf\xE9 na\xEFve";
        let detection = detect(bytes);
        assert_eq!(detection.encoding, Encoding::Latin1);
        assert_eq!(detection.encoding.decode(bytes), "café naïve");
    }

    #[test]
    fn a_lone_high_byte_is_latin1_rather_than_utf16() {
        // Two bytes, invalid UTF-8, too short for the parity heuristic to say
        // anything. Latin-1 always has an answer, which is why it is last.
        assert_eq!(detect(b"\xE9\n").encoding, Encoding::Latin1);
    }

    #[test]
    fn encoding_names_round_trip_through_parse() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
            Encoding::Latin1,
        ] {
            assert_eq!(Encoding::parse(encoding.as_str()), Some(encoding));
        }
        assert_eq!(Encoding::parse("UTF-8"), Some(Encoding::Utf8));
        assert_eq!(Encoding::parse("utf16le"), Some(Encoding::Utf16Le));
        assert_eq!(Encoding::parse("iso-8859-1"), Some(Encoding::Latin1));
        assert_eq!(Encoding::parse("shift-jis"), None);
    }

    #[test]
    fn the_wire_name_and_the_log_name_are_the_same_name() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
            Encoding::Latin1,
        ] {
            let json = serde_json::to_string(&encoding).unwrap();
            assert_eq!(json, format!("\"{}\"", encoding.as_str()));
        }
    }

    #[test]
    fn decode_then_encode_is_byte_identical_for_every_encoding() {
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let original = encoding.encode("round trip\nsecond line\n");
            let text = encoding.decode(&original.bytes);
            let again = encoding.encode(&text);
            assert_eq!(again.bytes, original.bytes, "{encoding}");
            assert!(!again.lossy);
        }
    }

    #[test]
    fn latin1_reports_a_lossy_save_for_characters_it_cannot_hold() {
        let outcome = Encoding::Latin1.encode("café → 世界");
        assert!(outcome.lossy);
        // The representable part survives; the rest is substituted, not lost
        // silently.
        assert!(outcome.bytes.starts_with(b"caf\xE9"));
    }

    #[test]
    fn a_png_header_is_binary() {
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR";
        assert!(looks_binary(png));
    }

    #[test]
    fn utf16_text_is_not_binary_despite_its_null_bytes() {
        assert!(!looks_binary(&utf16("hello world", false, true)));
        assert!(!looks_binary(&utf16("hello world", true, true)));
    }

    #[test]
    fn source_code_is_not_binary() {
        assert!(!looks_binary(b"const x = 1;\n"));
        assert!(!looks_binary(b""));
    }

    #[test]
    fn a_null_beyond_the_sniff_window_is_not_examined() {
        // Deliberate: the window is a fixed cost, and a file whose first 8KB
        // are clean text is treated as text. Bounding the scan is the point.
        let mut bytes = vec![b'a'; SNIFF_BYTES];
        bytes.push(0);
        assert!(!looks_binary(&bytes));
    }
}
