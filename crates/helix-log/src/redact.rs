//! Secret redaction and PII containment (REQ-OBS-001.10, .11, REQ-SEC-002.5).
//!
//! Redaction runs in [`crate::Logger::log`], *before* a record reaches any
//! sink. That ordering is the whole point: a sink added later (the crash
//! reporter in Task 13.1, a telemetry exporter, a plugin's developer panel)
//! inherits redaction by construction instead of by remembering to ask for
//! it. There is no code path that writes an unredacted record anywhere.
//!
//! Three mechanisms, in increasing order of confidence:
//!
//! 1. **Registered values.** Anything the secret service (Task 1.12) hands
//!    out is registered here and replaced literally wherever it appears.
//!    This is the only mechanism that is exact rather than heuristic, and it
//!    is why `log.redact` exists as an API rather than only as a scanner.
//! 2. **Key names.** A field (or a `key=value` pair inside a message) whose
//!    name looks like a credential has its value replaced regardless of what
//!    the value looks like.
//! 3. **Value shapes.** Known token prefixes (`sk-`, `ghp_`, `AKIA`, …),
//!    `Authorization: Bearer …` headers, URL userinfo, and PEM private key
//!    blocks are recognized by shape.
//!
//! Deliberately *not* implemented: entropy scoring. A high-entropy string is
//! as likely to be a content hash, a workspace key, or a temp path as a
//! token, and redacting file paths would break the one kind of PII the
//! requirement explicitly permits (REQ-OBS-001.10: paths yes, contents
//! never).
//!
//! The scanners are hand-written rather than regex-based. Every pattern here
//! is a literal prefix or a delimiter walk, which is a few lines each, and
//! it keeps a regex engine out of the dependency graph of the crate that
//! every other crate logs through.

use std::sync::RwLock;

use crate::record::{Fields, LogRecord};

/// What a redacted value is replaced with. Kept short and unmistakable so a
/// user reading a log knows something was removed rather than wondering
/// whether the value was empty.
pub const REDACTED: &str = "[redacted]";

/// What a field carrying file contents is replaced with (REQ-OBS-001.10).
pub const OMITTED_CONTENT: &str = "[omitted: log records never carry file contents]";

/// Longest string retained in a field value. A field is metadata about an
/// operation, not a payload; anything longer is almost certainly a buffer
/// that leaked into a diagnostic, so it is truncated rather than written to
/// disk in full.
pub const MAX_FIELD_CHARS: usize = 2_048;

/// Field and parameter names whose *value* is a credential regardless of its
/// shape. Matched case-insensitively, and as a substring so `openai_api_key`
/// and `gitCredential` are both caught.
const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "authorization",
    "credential",
    "private_key",
    "privatekey",
    "access_key",
    "accesskey",
    "client_secret",
    "session_key",
    "passphrase",
];

/// Field names that would carry file or buffer contents. Redaction is not
/// about secrecy here; it is REQ-OBS-001.10's "contents never" rule made
/// mechanical, so a well-meaning `fields: { content: … }` cannot put a
/// user's source file into a log they later attach to a bug report.
const CONTENT_KEY_NAMES: &[&str] = &[
    "content",
    "contents",
    "body",
    "file_content",
    "file_contents",
    "buffer",
    "clipboard",
];

/// Token prefixes that identify a credential by shape alone.
const TOKEN_PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "sk_test_",
    "pk_live_",
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xapp-",
    "AKIA",
    "ASIA",
    "AIza",
    "hf_",
    "npm_",
    "dckr_pat_",
];

/// Removes secrets and file contents from a record.
///
/// Cheap to share: the only mutable state is the registered-secret list,
/// behind an `RwLock` that is read-locked once per record.
#[derive(Debug, Default)]
pub struct Redactor {
    /// Exact values known to be secret, longest first so a token that
    /// contains another registered value is replaced whole.
    registered: RwLock<Vec<String>>,
}

impl Redactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an exact secret value, so it is replaced wherever it
    /// appears even in text no heuristic would flag.
    ///
    /// Very short values are ignored: registering a two-character secret
    /// would redact half of every message and teach users to distrust the
    /// log.
    pub fn register_secret(&self, value: impl Into<String>) {
        let value = value.into();
        if value.chars().count() < 6 {
            return;
        }
        let mut registered = self.registered.write().unwrap();
        if registered.iter().any(|existing| existing == &value) {
            return;
        }
        registered.push(value);
        // Longest first, so `abc123456789` is replaced before a registered
        // `abc123` swallows its prefix and leaves the tail exposed.
        registered.sort_by_key(|value| std::cmp::Reverse(value.len()));
    }

    pub fn forget_secret(&self, value: &str) {
        self.registered.write().unwrap().retain(|v| v != value);
    }

    pub fn registered_count(&self) -> usize {
        self.registered.read().unwrap().len()
    }

    /// Redact a record in place: message, field values, and field names that
    /// carry contents.
    pub fn redact_record(&self, record: &mut LogRecord) {
        record.message = self.redact_text(&record.message);
        record.fields = self.redact_fields(&record.fields);
    }

    /// Redact free text: registered values, then key/value pairs, then
    /// known token shapes.
    pub fn redact_text(&self, text: &str) -> String {
        let mut out = self.replace_registered(text);
        out = redact_pem_blocks(&out);
        // Shape-based scanning runs before key-based scanning so
        // `Authorization: Bearer <token>` loses the token and keeps the
        // scheme name, rather than the key scanner eating the word
        // "Bearer" and leaving the token behind it untouched.
        out = redact_token_prefixes(&out);
        out = redact_keyed_assignments(&out);
        out = redact_url_userinfo(&out);
        out
    }

    fn redact_fields(&self, fields: &Fields) -> Fields {
        let mut out = Fields::new();
        for (key, value) in fields {
            out.insert(key.clone(), self.redact_value(key, value));
        }
        out
    }

    fn redact_value(&self, key: &str, value: &serde_json::Value) -> serde_json::Value {
        if is_content_key(key) {
            return serde_json::Value::String(OMITTED_CONTENT.to_string());
        }
        if is_secret_key(key) {
            return serde_json::Value::String(REDACTED.to_string());
        }
        match value {
            serde_json::Value::String(s) => {
                serde_json::Value::String(truncate(&self.redact_text(s)))
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(
                items
                    .iter()
                    .map(|item| self.redact_value(key, item))
                    .collect(),
            ),
            serde_json::Value::Object(map) => {
                let mut nested = serde_json::Map::new();
                for (nested_key, nested_value) in map {
                    nested.insert(
                        nested_key.clone(),
                        self.redact_value(nested_key, nested_value),
                    );
                }
                serde_json::Value::Object(nested)
            }
            other => other.clone(),
        }
    }

    fn replace_registered(&self, text: &str) -> String {
        let registered = self.registered.read().unwrap();
        if registered.is_empty() {
            return text.to_string();
        }
        let mut out = text.to_string();
        for secret in registered.iter() {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), REDACTED);
            }
        }
        out
    }
}

/// True when a key name means "this value is a credential".
pub fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    SECRET_KEY_FRAGMENTS
        .iter()
        .any(|fragment| lowered.contains(fragment))
}

/// True when a key name means "this value is file or buffer content".
pub fn is_content_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    CONTENT_KEY_NAMES.iter().any(|name| lowered == *name)
}

fn truncate(value: &str) -> String {
    if value.chars().count() <= MAX_FIELD_CHARS {
        return value.to_string();
    }
    let kept: String = value.chars().take(MAX_FIELD_CHARS).collect();
    format!("{kept}…[truncated]")
}

/// Replace `-----BEGIN … PRIVATE KEY-----` … `-----END … -----` blocks.
///
/// Only private material is redacted. A certificate is public, and is
/// occasionally the thing being debugged.
fn redact_pem_blocks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("-----BEGIN") {
        let is_private = rest[start..]
            .lines()
            .next()
            .map(|line| line.contains("PRIVATE KEY"))
            .unwrap_or(false);
        if !is_private {
            let advance = start + "-----BEGIN".len();
            out.push_str(&rest[..advance]);
            rest = &rest[advance..];
            continue;
        }

        out.push_str(&rest[..start]);
        out.push_str(REDACTED);
        rest = match rest[start..].find("-----END") {
            // Everything from BEGIN through the end of the closing armour
            // line goes; the newline is kept so surrounding text keeps its
            // shape.
            Some(end_rel) => {
                let tail = &rest[start + end_rel..];
                match tail.find('\n') {
                    Some(newline) => &tail[newline..],
                    None => "",
                }
            }
            // An unterminated block is still a key; drop the remainder
            // rather than guessing where it ends.
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Replace the value in `key=value`, `key: value`, and `"key":"value"` when
/// the key looks like a credential.
fn redact_keyed_assignments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;

    while index < bytes.len() {
        // Walk to the end of the next identifier-ish run.
        if !is_key_char(bytes[index]) {
            out.push(bytes[index] as char);
            index += 1;
            continue;
        }
        let key_start = index;
        while index < bytes.len() && is_key_char(bytes[index]) {
            index += 1;
        }
        let key = &text[key_start..index];
        out.push_str(key);

        if !is_secret_key(key) {
            continue;
        }

        // Optional closing quote, whitespace, then a separator.
        let mut cursor = index;
        cursor = skip_while(bytes, cursor, |b| b == b'"' || b == b'\'' || b == b' ');
        if cursor >= bytes.len() || (bytes[cursor] != b'=' && bytes[cursor] != b':') {
            continue;
        }
        let separator_end = cursor + 1;
        let value_start = skip_while(bytes, separator_end, |b| {
            b == b' ' || b == b'"' || b == b'\''
        });
        if value_start >= bytes.len() || is_value_terminator(bytes[value_start]) {
            continue;
        }
        let mut value_end = value_start;
        while value_end < bytes.len() && !is_value_terminator(bytes[value_end]) {
            value_end += 1;
        }

        // A value an earlier pass already handled, or the scheme word of an
        // `Authorization: Bearer …` header whose token is already gone, is
        // left as it is: replacing it again would only add noise.
        let value = &text[value_start..value_end];
        // Compared against the placeholder's opening text rather than the
        // whole placeholder, because the value scan stops at its `]`.
        if value.starts_with("[redacted") || value.eq_ignore_ascii_case("bearer") {
            out.push_str(&text[index..value_end]);
            index = value_end;
            continue;
        }

        // Everything between the key and the value is structural and is
        // reproduced verbatim; only the value itself is replaced.
        out.push_str(&text[index..value_start]);
        out.push_str(REDACTED);
        index = value_end;
    }

    out
}

fn is_key_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_value_terminator(b: u8) -> bool {
    matches!(
        b,
        b' ' | b'"' | b'\'' | b',' | b'}' | b']' | b'&' | b'\n' | b'\r' | b'\t' | b';'
    )
}

fn skip_while(bytes: &[u8], mut index: usize, predicate: impl Fn(u8) -> bool) -> usize {
    while index < bytes.len() && predicate(bytes[index]) {
        index += 1;
    }
    index
}

/// Replace the password in `scheme://user:password@host`.
fn redact_url_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme_end) = rest.find("://") {
        let authority_start = scheme_end + 3;
        let authority = &rest[authority_start..];
        let authority_end = authority
            .find(['/', ' ', '"', '\n'])
            .unwrap_or(authority.len());
        let authority = &authority[..authority_end];

        match authority
            .find('@')
            .and_then(|at| authority[..at].find(':').map(|colon| (colon + 1, at)))
        {
            Some((password_start, at)) => {
                out.push_str(&rest[..authority_start + password_start]);
                out.push_str(REDACTED);
                rest = &rest[authority_start + at..];
            }
            None => {
                out.push_str(&rest[..authority_start]);
                rest = &rest[authority_start..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Replace `Bearer <token>` and anything starting with a known token
/// prefix.
fn redact_token_prefixes(text: &str) -> String {
    let mut out = redact_bearer(text);
    for prefix in TOKEN_PREFIXES {
        out = redact_prefixed(&out, prefix);
    }
    out
}

fn redact_bearer(text: &str) -> String {
    let lowered = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    while let Some(found) = lowered[index..].find("bearer ") {
        let start = index + found;
        let value_start = start + "bearer ".len();
        out.push_str(&text[index..value_start]);
        let bytes = text.as_bytes();
        let mut value_end = value_start;
        while value_end < bytes.len() && !is_value_terminator(bytes[value_end]) {
            value_end += 1;
        }
        if value_end > value_start {
            out.push_str(REDACTED);
        }
        index = value_end;
    }
    out.push_str(&text[index..]);
    out
}

fn redact_prefixed(text: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    while let Some(found) = text[index..].find(prefix) {
        let start = index + found;
        // Must begin a token, not sit inside a word, so `task-` inside
        // `subtask-1` is not mistaken for a credential.
        let preceded_by_key_char = start > 0 && is_key_char(text.as_bytes()[start - 1]);
        let mut end = start + prefix.len();
        let bytes = text.as_bytes();
        while end < bytes.len() && is_key_char(bytes[end]) {
            end += 1;
        }
        let token_body = end - (start + prefix.len());
        // A bare prefix with nothing after it is not a token.
        if preceded_by_key_char || token_body < 8 {
            out.push_str(&text[index..end.max(start + prefix.len())]);
            index = end.max(start + prefix.len());
            continue;
        }
        out.push_str(&text[index..start]);
        out.push_str(REDACTED);
        index = end;
    }
    out.push_str(&text[index..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{LogLevel, to_field};

    fn redactor() -> Redactor {
        Redactor::new()
    }

    #[test]
    fn a_registered_secret_is_replaced_wherever_it_appears() {
        let r = redactor();
        r.register_secret("hunter2-is-my-password");
        let out = r.redact_text("connecting with hunter2-is-my-password now");
        assert!(!out.contains("hunter2"));
        assert!(out.contains(REDACTED));
    }

    #[test]
    fn a_very_short_registered_value_is_ignored_to_keep_logs_readable() {
        let r = redactor();
        r.register_secret("abc");
        assert_eq!(r.registered_count(), 0);
        assert_eq!(r.redact_text("abc def"), "abc def");
    }

    #[test]
    fn a_field_named_like_a_credential_is_redacted_whatever_its_value() {
        let r = redactor();
        let mut record = LogRecord::new(LogLevel::Info, "ai", "provider configured")
            .with_field("openai_api_key", to_field("totally-innocent-looking"))
            .with_field("model", to_field("gpt-5"));
        r.redact_record(&mut record);

        assert_eq!(record.fields["openai_api_key"], REDACTED);
        assert_eq!(record.fields["model"], "gpt-5", "innocent fields survive");
    }

    #[test]
    fn a_nested_credential_field_is_redacted() {
        let r = redactor();
        let mut record = LogRecord::new(LogLevel::Info, "git", "clone").with_field(
            "remote",
            serde_json::json!({ "url": "https://example.com", "password": "s3cret-value" }),
        );
        r.redact_record(&mut record);
        assert_eq!(record.fields["remote"]["password"], REDACTED);
        assert_eq!(record.fields["remote"]["url"], "https://example.com");
    }

    #[test]
    fn a_content_field_is_omitted_because_logs_never_carry_file_contents() {
        let r = redactor();
        let mut record = LogRecord::new(LogLevel::Debug, "fs", "file saved")
            .with_field("path", to_field("/home/user/project/src/main.rs"))
            .with_field("content", to_field("fn main() { /* private */ }"));
        r.redact_record(&mut record);

        assert_eq!(record.fields["content"], OMITTED_CONTENT);
        assert_eq!(
            record.fields["path"], "/home/user/project/src/main.rs",
            "paths are permitted PII (REQ-OBS-001.10)"
        );
    }

    #[test]
    fn an_over_long_field_value_is_truncated_rather_than_written_in_full() {
        let r = redactor();
        let long = "x".repeat(MAX_FIELD_CHARS * 2);
        let mut record =
            LogRecord::new(LogLevel::Debug, "fs", "read").with_field("blob", to_field(&long));
        r.redact_record(&mut record);

        let value = record.fields["blob"].as_str().unwrap();
        assert!(value.ends_with("…[truncated]"));
        assert!(value.chars().count() < long.chars().count());
    }

    #[test]
    fn a_keyed_assignment_inside_a_message_is_redacted() {
        let r = redactor();
        assert_eq!(
            r.redact_text("spawning helper --password=letmein --verbose"),
            format!("spawning helper --password={REDACTED} --verbose")
        );
        assert_eq!(
            r.redact_text("headers: { authorization: abcdef12345 }"),
            format!("headers: {{ authorization: {REDACTED} }}")
        );
    }

    #[test]
    fn a_json_shaped_credential_inside_a_message_is_redacted() {
        let r = redactor();
        let out = r.redact_text(r#"{"api_key":"sk-abcdefghijklmnop","model":"gpt-5"}"#);
        assert!(!out.contains("abcdefghijklmnop"), "{out}");
        assert!(out.contains("gpt-5"));
    }

    #[test]
    fn a_bearer_token_is_redacted() {
        let r = redactor();
        let out = r.redact_text("GET /v1/models Authorization: Bearer eyJhbGciOiJIUzI1NiJ9");
        assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9"), "{out}");
        assert!(out.contains(REDACTED));
    }

    #[test]
    fn a_token_recognized_by_prefix_is_redacted_even_without_a_key_name() {
        let r = redactor();
        for token in [
            "ghp_1234567890abcdefghij",
            "AKIAIOSFODNN7EXAMPLE",
            "glpat-abcdefghij1234567890",
        ] {
            let out = r.redact_text(&format!("using {token} to authenticate"));
            assert!(!out.contains(token), "{token} survived redaction: {out}");
        }
    }

    #[test]
    fn an_ordinary_word_sharing_a_token_prefix_is_left_alone() {
        let r = redactor();
        // "sk-" appears inside a hyphenated path segment; not a credential.
        let text = "opened /home/user/task-skeleton/notes.md";
        assert_eq!(r.redact_text(text), text);
    }

    #[test]
    fn url_credentials_are_redacted_but_the_host_survives() {
        let r = redactor();
        let out = r.redact_text("fetching https://alice:s3cr3tpass@git.example.com/repo.git");
        assert!(!out.contains("s3cr3tpass"), "{out}");
        assert!(out.contains("git.example.com"));
        assert!(out.contains("alice"));
    }

    #[test]
    fn a_url_without_credentials_is_untouched() {
        let r = redactor();
        let text = "fetching https://git.example.com/repo.git";
        assert_eq!(r.redact_text(text), text);
    }

    #[test]
    fn a_pem_private_key_block_is_redacted_whole() {
        let r = redactor();
        let text = "loaded key:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEow==\n-----END RSA PRIVATE KEY-----\ndone";
        let out = r.redact_text(text);
        assert!(!out.contains("MIIEow=="), "{out}");
        assert!(out.contains("done"));
    }

    #[test]
    fn a_public_certificate_block_is_not_redacted() {
        let r = redactor();
        let text = "-----BEGIN CERTIFICATE-----\nMIIB==\n-----END CERTIFICATE-----";
        assert!(r.redact_text(text).contains("MIIB=="));
    }

    #[test]
    fn plain_text_is_returned_unchanged() {
        let r = redactor();
        let text = "started language server for typescript in 1200ms";
        assert_eq!(r.redact_text(text), text);
    }

    #[test]
    fn forgetting_a_secret_stops_replacing_it() {
        let r = redactor();
        r.register_secret("rotate-me-please");
        r.forget_secret("rotate-me-please");
        assert_eq!(r.registered_count(), 0);
        assert_eq!(r.redact_text("rotate-me-please"), "rotate-me-please");
    }
}
