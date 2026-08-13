//! Namespace rules for secret isolation (REQ-SEC-002.3).

use crate::error::SecretError;

/// Who is asking for a secret, which determines namespace access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretCaller {
    /// Kernel subsystems (`helix`, `git`, …).
    Kernel,
    /// A plugin may only touch its own namespace (`plugin.{id}`).
    Plugin { plugin_id: String },
    /// The settings UI stores provider credentials under `helix`.
    SettingsUi,
}

impl SecretCaller {
    pub fn allows_namespace(&self, namespace: &str) -> bool {
        match self {
            Self::Kernel => true,
            Self::SettingsUi => namespace == HELIX_NAMESPACE || namespace == GIT_NAMESPACE,
            Self::Plugin { plugin_id } => {
                namespace == format!("{PLUGIN_NAMESPACE_PREFIX}.{plugin_id}")
            }
        }
    }

    pub fn deny_if_needed(&self, namespace: &str) -> Result<(), SecretError> {
        if self.allows_namespace(namespace) {
            Ok(())
        } else {
            Err(SecretError::NamespaceDenied {
                namespace: namespace.to_string(),
            })
        }
    }
}

pub const HELIX_NAMESPACE: &str = "helix";
pub const GIT_NAMESPACE: &str = "git";
pub const PLUGIN_NAMESPACE_PREFIX: &str = "plugin";

/// Validate a secret name segment: non-empty, no slashes, reasonable length.
pub fn validate_name(name: &str) -> Result<(), SecretError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(SecretError::InvalidName("name must not be empty".into()));
    }
    if trimmed.len() > 256 {
        return Err(SecretError::InvalidName("name is too long".into()));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.chars().any(char::is_control) {
        return Err(SecretError::InvalidName(
            "name must not contain path separators or control characters".into(),
        ));
    }
    Ok(())
}

/// Validate a namespace segment.
pub fn validate_namespace(namespace: &str) -> Result<(), SecretError> {
    let trimmed = namespace.trim();
    if trimmed.is_empty() {
        return Err(SecretError::InvalidName(
            "namespace must not be empty".into(),
        ));
    }
    if trimmed.len() > 128 {
        return Err(SecretError::InvalidName("namespace is too long".into()));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.chars().any(char::is_control) {
        return Err(SecretError::InvalidName(
            "namespace must not contain path separators or control characters".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_are_confined_to_their_own_namespace() {
        let caller = SecretCaller::Plugin {
            plugin_id: "acme".into(),
        };
        assert!(caller.allows_namespace("plugin.acme"));
        assert!(!caller.allows_namespace("plugin.other"));
        assert!(!caller.allows_namespace("helix"));
    }

    #[test]
    fn the_settings_ui_may_write_kernel_and_git_namespaces_only() {
        let caller = SecretCaller::SettingsUi;
        assert!(caller.allows_namespace("helix"));
        assert!(caller.allows_namespace("git"));
        assert!(!caller.allows_namespace("plugin.acme"));
    }
}
