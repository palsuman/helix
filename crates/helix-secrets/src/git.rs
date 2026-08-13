//! Git credential helper protocol (REQ-SEC-002.8).

use crate::backend::{CompositeBackend, KeyringBackend, SecretBackend};
use crate::error::SecretError;
use crate::namespace::{GIT_NAMESPACE, HELIX_NAMESPACE};
use zeroize::Zeroize;

/// Parse `key=value` lines from git-credential stdin.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GitCredential {
    pub protocol: Option<String>,
    pub host: Option<String>,
    pub path: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Drop for GitCredential {
    fn drop(&mut self) {
        if let Some(password) = &mut self.password {
            password.zeroize();
        }
    }
}

impl GitCredential {
    pub fn parse(input: &str) -> Self {
        let mut cred = Self::default();
        for line in input.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "protocol" => cred.protocol = Some(value.to_string()),
                "host" => cred.host = Some(value.to_string()),
                "path" => cred.path = Some(value.to_string()),
                "username" => cred.username = Some(value.to_string()),
                "password" => cred.password = Some(value.to_string()),
                _ => {}
            }
        }
        cred
    }

    pub fn storage_name(&self) -> Option<String> {
        let host = self.host.as_deref()?;
        Some(match self.protocol.as_deref() {
            Some(protocol) => format!("{protocol}://{host}"),
            None => host.to_string(),
        })
    }

    pub fn to_protocol_output(&self) -> String {
        let mut lines = Vec::new();
        if let Some(protocol) = &self.protocol {
            lines.push(format!("protocol={protocol}"));
        }
        if let Some(host) = &self.host {
            lines.push(format!("host={host}"));
        }
        if let Some(path) = &self.path {
            lines.push(format!("path={path}"));
        }
        if let Some(username) = &self.username {
            lines.push(format!("username={username}"));
        }
        if let Some(password) = &self.password {
            lines.push(format!("password={password}"));
        }
        if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        }
    }
}

pub fn handle_git_credential(action: &str, input: &str) -> Result<String, SecretError> {
    let cred = GitCredential::parse(input);
    let backend = production_backend();
    match action {
        "get" | "fill" => fill(&backend, &cred),
        "store" => store(&backend, &cred),
        "erase" => erase(&backend, &cred),
        other => Err(SecretError::storage(format!(
            "unknown git-credential action '{other}'"
        ))),
    }
}

fn fill(backend: &dyn SecretBackend, cred: &GitCredential) -> Result<String, SecretError> {
    let name = cred
        .storage_name()
        .ok_or_else(|| SecretError::storage("git credential request missing host"))?;
    let stored = backend.get(GIT_NAMESPACE, &name)?;
    let (username, password) = stored
        .split_once(':')
        .map(|(user, pass)| (user.to_string(), pass.to_string()))
        .unwrap_or_else(|| ("git".to_string(), stored.clone()));
    Ok(GitCredential {
        protocol: cred.protocol.clone(),
        host: cred.host.clone(),
        path: cred.path.clone(),
        username: Some(username),
        password: Some(password),
    }
    .to_protocol_output())
}

fn store(backend: &dyn SecretBackend, cred: &GitCredential) -> Result<String, SecretError> {
    let name = cred
        .storage_name()
        .ok_or_else(|| SecretError::storage("git credential store missing host"))?;
    let username = cred
        .username
        .as_deref()
        .ok_or_else(|| SecretError::storage("git credential store missing username"))?;
    let password = cred
        .password
        .as_deref()
        .ok_or_else(|| SecretError::storage("git credential store missing password"))?;
    backend.store(GIT_NAMESPACE, &name, &format!("{username}:{password}"))?;
    Ok(String::new())
}

fn erase(backend: &dyn SecretBackend, cred: &GitCredential) -> Result<String, SecretError> {
    if let Some(name) = cred.storage_name() {
        let _ = backend.delete(GIT_NAMESPACE, &name);
    }
    Ok(String::new())
}

fn production_backend() -> CompositeBackend {
    let vault = crate::backend::default_vault_path()
        .unwrap_or_else(|| std::env::temp_dir().join("helix-secrets-vault.json"));
    CompositeBackend::new(vault)
}

/// Direct keyring access for the helper when the kernel is not running.
pub fn store_provider_key(name: &str, value: &str) -> Result<(), SecretError> {
    KeyringBackend::new().store(HELIX_NAMESPACE, name, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryBackend;

    #[test]
    fn git_store_and_fill_round_trip_in_memory() {
        let backend = MemoryBackend::new();
        let input = "protocol=https\nhost=github.com\nusername=alice\npassword=secret-token\n";
        store(&backend, &GitCredential::parse(input)).unwrap();
        let out = fill(
            &backend,
            &GitCredential::parse("protocol=https\nhost=github.com\n"),
        )
        .unwrap();
        assert!(out.contains("username=alice"));
        assert!(out.contains("password=secret-token"));
    }

    #[test]
    fn keyring_service_name_is_stable() {
        assert_eq!(crate::backend::KEYRING_SERVICE, "dev.helix.ide");
    }
}
