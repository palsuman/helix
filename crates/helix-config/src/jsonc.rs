//! JSON-with-comments parsing, with the parse error's location preserved.
//!
//! The design document specifies JSONC for settings files, because a settings
//! file people are expected to hand-edit without being able to annotate or
//! temporarily disable a line is hostile. `serde_json` does not accept
//! comments or trailing commas, so this module rewrites them to whitespace
//! *in place* before parsing.
//!
//! Rewriting rather than deleting is the point: every byte keeps its offset,
//! so the line and column `serde_json` reports for a syntax error are the line
//! and column in the file the user is looking at (REQ-CONFIG-001 failure mode:
//! "highlight the location").
//!
//! Not implemented: a full JSONC parser retaining comments for round-trip
//! writes. `config.set` reserializes the document, so authored comments are
//! lost on a programmatic write; that is a known limitation recorded in
//! [`crate::service::ConfigService::set`] rather than a silent one.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ts_rs::TS;

/// A syntax error in a settings file, with enough location to highlight it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct ConfigParseError {
    /// Absolute path of the file that failed to parse.
    pub path: String,
    pub message: String,
    /// 1-based line, as reported by the parser against the original text.
    pub line: u32,
    /// 1-based column.
    pub column: u32,
}

impl ConfigParseError {
    pub fn new(
        path: impl Into<String>,
        message: impl Into<String>,
        line: u32,
        column: u32,
    ) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
            line,
            column,
        }
    }
}

impl std::fmt::Display for ConfigParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.path, self.line, self.column, self.message
        )
    }
}

/// Replace `//` and `/* */` comments, and trailing commas, with spaces.
///
/// Newlines inside block comments are preserved so line numbers do not
/// shift. String literals are walked with escape handling, so a `//` inside
/// a string (a URL, a Windows path) is left alone.
pub fn blank_comments(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0usize;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                // Copy the string literal verbatim, including escapes.
                out.push(bytes[index]);
                index += 1;
                while index < bytes.len() {
                    let byte = bytes[index];
                    out.push(byte);
                    index += 1;
                    if byte == b'\\' && index < bytes.len() {
                        out.push(bytes[index]);
                        index += 1;
                    } else if byte == b'"' {
                        break;
                    }
                }
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'/' => {
                while index < bytes.len() && bytes[index] != b'\n' {
                    out.push(b' ');
                    index += 1;
                }
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
                let mut closed = false;
                while index < bytes.len() {
                    if bytes[index] == b'\n' {
                        out.push(b'\n');
                        index += 1;
                        continue;
                    }
                    if bytes[index] == b'*' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
                        out.push(b' ');
                        out.push(b' ');
                        index += 2;
                        closed = true;
                        break;
                    }
                    out.push(b' ');
                    index += 1;
                }
                if !closed {
                    // Unterminated block comment: the remainder is already
                    // blanked, and the parser will report the resulting
                    // truncation at a location inside the real file.
                }
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }

    let out = blank_trailing_commas(out);
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

/// Replace a comma that is followed only by whitespace and a `}` or `]`.
///
/// One ASCII byte is replaced by one ASCII byte, so UTF-8 boundaries in the
/// surrounding text are untouched.
fn blank_trailing_commas(mut bytes: Vec<u8>) -> Vec<u8> {
    let mut in_string = false;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' if !in_string => in_string = true,
            b'\\' if in_string => index += 1,
            b'"' if in_string => in_string = false,
            b',' if !in_string => {
                let mut look = index + 1;
                while look < bytes.len() && bytes[look].is_ascii_whitespace() {
                    look += 1;
                }
                if look < bytes.len() && (bytes[look] == b'}' || bytes[look] == b']') {
                    bytes[index] = b' ';
                }
            }
            _ => {}
        }
        index += 1;
    }
    bytes
}

/// Parse a settings file body into a top-level object.
///
/// A body that parses but is not an object is an error rather than a silent
/// empty layer: `[1, 2]` in `settings.json` is a mistake worth reporting.
/// An empty (or whitespace/comment-only) body is an empty layer, which is
/// what a freshly created file is.
pub fn parse_object(path: &str, body: &str) -> Result<Map<String, Value>, ConfigParseError> {
    let blanked = blank_comments(body);
    if blanked.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(&blanked) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(other) => Err(ConfigParseError::new(
            path,
            format!(
                "settings must be a JSON object, found {}",
                describe_shape(&other)
            ),
            1,
            1,
        )),
        Err(error) => Err(ConfigParseError::new(
            path,
            error.to_string(),
            error.line() as u32,
            error.column() as u32,
        )),
    }
}

fn describe_shape(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_comments_are_removed_without_shifting_anything() {
        let input = "{\n  // a comment\n  \"a\": 1\n}";
        let blanked = blank_comments(input);
        assert_eq!(blanked.len(), input.len());
        assert_eq!(blanked.lines().count(), input.lines().count());
        assert!(!blanked.contains("comment"));
        assert_eq!(
            serde_json::from_str::<Value>(&blanked).unwrap(),
            serde_json::json!({ "a": 1 })
        );
    }

    #[test]
    fn block_comments_keep_their_newlines_so_line_numbers_survive() {
        let input = "{\n/* one\n   two */\n  \"a\": 1\n}";
        let blanked = blank_comments(input);
        assert_eq!(blanked.lines().count(), input.lines().count());
        assert!(serde_json::from_str::<Value>(&blanked).is_ok());
    }

    #[test]
    fn a_comment_marker_inside_a_string_is_left_alone() {
        let body = r#"{ "url": "https://example.com/a", "path": "C:\\a\\b" }"#;
        let parsed = parse_object("settings.json", body).unwrap();
        assert_eq!(parsed["url"], "https://example.com/a");
        assert_eq!(parsed["path"], r"C:\a\b");
    }

    #[test]
    fn trailing_commas_are_accepted() {
        let parsed =
            parse_object("settings.json", "{ \"a\": [1, 2,], \"b\": { \"c\": 1, }, }").unwrap();
        assert_eq!(parsed["a"], serde_json::json!([1, 2]));
        assert_eq!(parsed["b"]["c"], 1);
    }

    #[test]
    fn a_comma_inside_a_string_is_not_treated_as_trailing() {
        let parsed = parse_object("settings.json", r#"{ "a": "x,]" }"#).unwrap();
        assert_eq!(parsed["a"], "x,]");
    }

    #[test]
    fn an_empty_or_comment_only_body_is_an_empty_layer() {
        assert!(parse_object("s.json", "").unwrap().is_empty());
        assert!(
            parse_object("s.json", "  \n// nothing yet\n")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_syntax_error_reports_the_line_and_column_in_the_original_file() {
        let body = "{\n  \"a\": 1\n  \"b\": 2\n}";
        let error = parse_object("/tmp/settings.json", body).unwrap_err();
        assert_eq!(error.line, 3, "{error}");
        assert!(error.column >= 3, "{error}");
        assert_eq!(error.path, "/tmp/settings.json");
    }

    #[test]
    fn a_non_object_document_is_reported_rather_than_silently_ignored() {
        let error = parse_object("s.json", "[1, 2]").unwrap_err();
        assert!(error.message.contains("must be a JSON object"), "{error}");
    }
}
