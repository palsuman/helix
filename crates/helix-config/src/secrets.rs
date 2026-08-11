//! Refusing secrets in settings files (REQ-CONFIG-001.12, REQ-SEC-002.4).
//!
//! Settings files are shareable by design: a workspace one is committed, and a
//! user one is the first thing people paste into a gist when asking for help.
//! A credential in either is a credential leaked, so the configuration service
//! refuses to write one and ignores one it finds on load.
//!
//! Detection reuses `helix-log`'s redaction heuristics rather than growing a
//! second, subtly different set of patterns. That crate already has to decide
//! "is this a credential?" for every log record, and having one answer means a
//! value rejected here is also a value that would never appear in a log.
//!
//! Two signals, both from the same source of truth:
//!
//! - **Key name.** Any dotted segment that reads like a credential
//!   (`password`, `token`, `apiKey`, …) condemns the value whatever it looks
//!   like.
//! - **Value shape.** A string that redaction would rewrite (a `sk-…` token,
//!   a `Bearer …` header, URL userinfo, a PEM private key block) is treated as
//!   a credential.
//!
//! The intended replacement is always the same: store the credential in the OS
//! keychain (Task 1.12) and reference it from settings by name, which is what
//! [`SecretFinding::guidance`] tells the user.

use serde_json::Value;

use helix_log::Redactor;
use helix_log::redact::is_secret_key;

/// Why a value was treated as a credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSignal {
    /// The key (or one of its segments) names a credential.
    KeyName,
    /// The value has the shape of a known credential.
    ValueShape,
}

/// A credential found in a settings document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretFinding {
    /// Dotted key of the offending value, including any nested path inside an
    /// object-valued setting.
    pub key: String,
    pub signal: SecretSignal,
}

impl SecretFinding {
    /// A message naming the offending key and what to do instead. Never
    /// includes the value: an error message about a leaked secret that quotes
    /// the secret has leaked it again, into the log.
    pub fn guidance(&self) -> String {
        let reason = match self.signal {
            SecretSignal::KeyName => "its name identifies it as a credential",
            SecretSignal::ValueShape => "its value has the shape of a credential",
        };
        format!(
            "'{}' was rejected because {reason}. Settings files are shareable, so credentials belong in the OS keychain and should be referenced from settings by name.",
            self.key
        )
    }
}

/// Scan a value for credentials, reporting every one found.
///
/// `key` is the dotted key the value sits at; nested findings extend it, so an
/// offending entry inside `ai.providers` is reported as
/// `ai.providers.openai.token` rather than as `ai.providers`.
pub fn scan(key: &str, value: &Value) -> Vec<SecretFinding> {
    let redactor = Redactor::new();
    let mut findings = Vec::new();
    scan_into(key, key, value, &redactor, &mut findings);
    findings
}

/// Whether a value contains anything that looks like a credential.
pub fn contains_secret(key: &str, value: &Value) -> bool {
    !scan(key, value).is_empty()
}

fn scan_into(
    full_key: &str,
    leaf_name: &str,
    value: &Value,
    redactor: &Redactor,
    findings: &mut Vec<SecretFinding>,
) {
    if key_is_credentialish(leaf_name) {
        findings.push(SecretFinding {
            key: full_key.to_string(),
            signal: SecretSignal::KeyName,
        });
        // No point descending: the whole subtree is refused with its parent.
        return;
    }

    match value {
        Value::String(text) => {
            if redactor.redact_text(text) != *text {
                findings.push(SecretFinding {
                    key: full_key.to_string(),
                    signal: SecretSignal::ValueShape,
                });
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let nested = format!("{full_key}[{index}]");
                scan_into(&nested, leaf_name, item, redactor, findings);
            }
        }
        Value::Object(map) => {
            for (child_key, child) in map {
                let nested = format!("{full_key}.{child_key}");
                scan_into(&nested, child_key, child, redactor, findings);
            }
        }
        _ => {}
    }
}

/// Whether any dotted segment of a key names a credential.
fn key_is_credentialish(key: &str) -> bool {
    key.split('.').any(is_secret_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_credential_shaped_key_is_rejected_whatever_its_value_looks_like() {
        let findings = scan("ai.openai.apiKey", &json!("nothing-suspicious-here"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signal, SecretSignal::KeyName);
        assert_eq!(findings[0].key, "ai.openai.apiKey");
    }

    #[test]
    fn a_credential_shaped_value_is_rejected_under_an_innocent_key() {
        let findings = scan("ai.defaultModel", &json!("sk-abcdefghijklmnopqrstuv"));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].signal, SecretSignal::ValueShape);
    }

    #[test]
    fn a_credential_nested_inside_an_object_setting_is_reported_by_its_full_path() {
        let findings = scan(
            "ai.providers",
            &json!({ "openai": { "endpoint": "https://api.openai.com", "token": "abc123456789" } }),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].key, "ai.providers.openai.token");
    }

    #[test]
    fn a_credential_inside_an_array_is_found() {
        let findings = scan(
            "some.list",
            &json!(["fine", "https://user:supersecret@example.com/x"]),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].key, "some.list[1]");
    }

    #[test]
    fn ordinary_settings_are_left_alone() {
        for (key, value) in [
            ("editor.fontSize", json!(14)),
            ("workbench.colorTheme", json!("Helix Dark")),
            (
                "files.exclude",
                json!({ "**/node_modules": true, "**/.git": true }),
            ),
            ("terminal.shellPath", json!("/usr/bin/zsh")),
            ("ai.providers", json!({ "openai": { "keyId": "work" } })),
        ] {
            assert!(
                !contains_secret(key, &value),
                "'{key}' should not be treated as a credential"
            );
        }
    }

    #[test]
    fn the_guidance_names_the_key_and_never_the_value() {
        let findings = scan("git.password", &json!("hunter2-the-real-one"));
        let guidance = findings[0].guidance();
        assert!(guidance.contains("git.password"));
        assert!(!guidance.contains("hunter2"));
        assert!(guidance.contains("keychain"));
    }
}
