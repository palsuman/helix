//! Line ending detection and normalisation (Task 1.7, REQ-ED-006.5).
//!
//! The editor works on `\n` internally and converts on the boundary. That is
//! the only arrangement in which "this file is CRLF" is a property of the file
//! rather than a property smeared through every buffer operation.
//!
//! Detection reports [`LineEnding::Mixed`] rather than picking a winner,
//! because a mixed file is usually a symptom (a bad merge, a tool that wrote
//! half the file) and silently normalising it on the next save would produce a
//! diff touching every line. The dominant style is still available via
//! [`EolInfo::dominant`] for the case where a save has to choose.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The line ending style of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    Lf,
    Crlf,
    /// Both styles present. Reported, not resolved.
    Mixed,
    /// No line break at all: a single-line file, or an empty one.
    None,
}

impl LineEnding {
    /// The bytes this style writes.
    ///
    /// [`LineEnding::Mixed`] and [`LineEnding::None`] have no sequence of
    /// their own, so both write LF. For `None` that is the platform-neutral
    /// default; for `Mixed` it means an explicit "normalise this file" action
    /// picks LF unless the caller resolved the ambiguity first.
    pub fn as_bytes(self) -> &'static [u8] {
        match self {
            LineEnding::Crlf => b"\r\n",
            _ => b"\n",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "lf",
            LineEnding::Crlf => "crlf",
            LineEnding::Mixed => "mixed",
            LineEnding::None => "none",
        }
    }

    /// Parse the `files.eol` setting. `auto` is not a style, so it is `None`
    /// here and means "keep whatever the file already had".
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "lf" | "\n" => Some(LineEnding::Lf),
            "crlf" | "\r\n" => Some(LineEnding::Crlf),
            _ => None,
        }
    }

    /// The style used by this platform, for a buffer with no file behind it.
    pub fn platform_default() -> Self {
        if cfg!(windows) {
            LineEnding::Crlf
        } else {
            LineEnding::Lf
        }
    }
}

impl std::fmt::Display for LineEnding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Per-file line ending report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct EolInfo {
    pub style: LineEnding,
    pub lf_count: u32,
    pub crlf_count: u32,
}

impl EolInfo {
    /// The majority style, for a caller that must write one or the other.
    /// Ties go to LF, matching what a new file gets.
    pub fn dominant(&self) -> LineEnding {
        match self.style {
            LineEnding::Mixed if self.crlf_count > self.lf_count => LineEnding::Crlf,
            LineEnding::Mixed => LineEnding::Lf,
            LineEnding::None => LineEnding::platform_default(),
            style => style,
        }
    }
}

/// Detect the line ending style of decoded text.
///
/// A lone `\r` (classic Mac) is counted as neither: it is vanishingly rare,
/// and treating it as a third style would put a third arm in every match in
/// the editor for the sake of files from 2001. It is left in the text as a
/// literal carriage return, which is at least visible and lossless.
pub fn detect(text: &str) -> EolInfo {
    let mut lf_count = 0u32;
    let mut crlf_count = 0u32;
    let bytes = text.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if index > 0 && bytes[index - 1] == b'\r' {
                crlf_count += 1;
            } else {
                lf_count += 1;
            }
        }
    }

    let style = match (lf_count, crlf_count) {
        (0, 0) => LineEnding::None,
        (0, _) => LineEnding::Crlf,
        (_, 0) => LineEnding::Lf,
        _ => LineEnding::Mixed,
    };

    EolInfo {
        style,
        lf_count,
        crlf_count,
    }
}

/// Convert every line ending to LF, for the editor's internal representation.
pub fn to_lf(text: &str) -> String {
    if !text.contains('\r') {
        // The overwhelmingly common case, and worth not allocating for.
        return text.to_string();
    }
    text.replace("\r\n", "\n")
}

/// Convert LF-normalised text to the requested style on the way to disk.
pub fn from_lf(text: &str, style: LineEnding) -> String {
    match style {
        LineEnding::Crlf => text.replace('\n', "\r\n"),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lf_only_text_is_lf() {
        let info = detect("a\nb\nc\n");
        assert_eq!(info.style, LineEnding::Lf);
        assert_eq!(info.lf_count, 3);
        assert_eq!(info.crlf_count, 0);
    }

    #[test]
    fn crlf_only_text_is_crlf() {
        let info = detect("a\r\nb\r\n");
        assert_eq!(info.style, LineEnding::Crlf);
        assert_eq!(info.crlf_count, 2);
        assert_eq!(info.lf_count, 0);
    }

    #[test]
    fn a_file_with_both_styles_is_reported_as_mixed_not_resolved() {
        let info = detect("a\r\nb\nc\r\n");
        assert_eq!(info.style, LineEnding::Mixed);
        assert_eq!(info.crlf_count, 2);
        assert_eq!(info.lf_count, 1);
        assert_eq!(info.dominant(), LineEnding::Crlf);
    }

    #[test]
    fn a_single_line_file_has_no_line_ending() {
        assert_eq!(detect("no newline here").style, LineEnding::None);
        assert_eq!(detect("").style, LineEnding::None);
    }

    #[test]
    fn a_mixed_file_with_more_lf_prefers_lf() {
        let info = detect("a\nb\nc\r\n");
        assert_eq!(info.style, LineEnding::Mixed);
        assert_eq!(info.dominant(), LineEnding::Lf);
    }

    #[test]
    fn normalisation_round_trips_through_the_detected_style() {
        for original in ["a\nb\n", "a\r\nb\r\n"] {
            let style = detect(original).style;
            let internal = to_lf(original);
            assert!(!internal.contains('\r'));
            assert_eq!(from_lf(&internal, style), original);
        }
    }

    #[test]
    fn a_lone_carriage_return_is_preserved_and_counted_as_no_style() {
        let info = detect("a\rb");
        assert_eq!(info.style, LineEnding::None);
        assert_eq!(to_lf("a\rb"), "a\rb");
    }

    #[test]
    fn the_eol_setting_parses_only_real_styles() {
        assert_eq!(LineEnding::parse("lf"), Some(LineEnding::Lf));
        assert_eq!(LineEnding::parse("CRLF"), Some(LineEnding::Crlf));
        assert_eq!(LineEnding::parse("auto"), None);
    }
}
