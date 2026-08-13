//! Encrypted on-disk fallback when the OS keychain is unavailable.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::backend::{SecretBackend, SecretEntry};
use crate::error::SecretError;

const VAULT_VERSION: u32 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const VAULT_VERIFIER: &[u8] = b"helix-secret-vault-verifier-v1";

#[derive(Debug, Serialize, Deserialize)]
struct VaultIndex {
    version: u32,
    /// Salt for the master-password key derivation, written on first unlock.
    salt: Option<String>,
    /// `None` records a keychain entry for portable listing. `Some` names an
    /// encrypted fallback blob.
    entries: BTreeMap<String, Option<String>>,
    #[serde(default)]
    verifier: Option<EncryptedBlob>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedBlob {
    version: u32,
    nonce: String,
    ciphertext: String,
}

pub struct EncryptedFileBackend {
    path: PathBuf,
    key: RwLock<Option<Zeroizing<[u8; 32]>>>,
    index: Mutex<VaultIndex>,
    load_error: Option<SecretError>,
}

impl EncryptedFileBackend {
    pub fn new(path: PathBuf) -> Self {
        let (index, load_error) = match load_index(&path) {
            Ok(index) => (index, None),
            Err(SecretError::NotFound { .. }) => (empty_index(), None),
            Err(error) => (empty_index(), Some(error)),
        };
        Self {
            path,
            key: RwLock::new(None),
            index: Mutex::new(index),
            load_error,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_unlocked(&self) -> bool {
        self.key.read().unwrap().is_some()
    }

    pub fn load_error(&self) -> Option<SecretError> {
        self.load_error.clone()
    }

    pub fn unlock(&self, master_password: &str) -> Result<(), SecretError> {
        self.ensure_loaded()?;
        if master_password.is_empty() {
            return Err(SecretError::InvalidMasterPassword);
        }
        let mut index = self.index.lock().unwrap();
        let salt = match index.salt.as_deref() {
            Some(encoded) => decode_blob_part(encoded)?,
            None => {
                let salt = random_bytes(SALT_LEN);
                index.salt = Some(encode_blob_part(&salt));
                save_index(&self.path, &index)?;
                salt
            }
        };
        let key = derive_key(master_password, &salt)?;
        if let Some(verifier) = &index.verifier {
            let plaintext = decrypt_with_key(&key, verifier, VAULT_VERIFIER)?;
            if plaintext != VAULT_VERIFIER {
                return Err(SecretError::InvalidMasterPassword);
            }
        } else if let Some((qualified, blob_name)) = index
            .entries
            .iter()
            .find_map(|(qualified, blob)| blob.as_ref().map(|blob| (qualified, blob)))
        {
            let dir = self
                .path
                .parent()
                .ok_or_else(|| SecretError::storage("vault path has no parent directory"))?;
            let blob = read_blob(dir, blob_name)?;
            decrypt_with_key(&key, &blob, qualified.as_bytes())?;
        }
        if index.verifier.is_none() {
            index.verifier = Some(encrypt_with_key(&key, VAULT_VERIFIER, VAULT_VERIFIER)?);
            save_index(&self.path, &index)?;
        }
        *self.key.write().unwrap() = Some(key);
        Ok(())
    }

    pub fn record_index(&self, namespace: &str, name: &str) -> Result<(), SecretError> {
        self.ensure_loaded()?;
        let mut index = self.index.lock().unwrap();
        index
            .entries
            .entry(qualified_key(namespace, name))
            .or_insert(None);
        save_index(&self.path, &index)
    }

    pub fn contains_index(&self, namespace: &str, name: &str) -> Result<bool, SecretError> {
        self.ensure_loaded()?;
        Ok(self
            .index
            .lock()
            .unwrap()
            .entries
            .contains_key(&qualified_key(namespace, name)))
    }

    fn ensure_loaded(&self) -> Result<(), SecretError> {
        match &self.load_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

impl SecretBackend for EncryptedFileBackend {
    fn kind(&self) -> crate::backend::BackendKind {
        crate::backend::BackendKind::EncryptedFile
    }

    fn store(&self, namespace: &str, name: &str, value: &str) -> Result<(), SecretError> {
        self.ensure_loaded()?;
        let key = self
            .key
            .read()
            .unwrap()
            .clone()
            .ok_or(SecretError::FallbackLocked)?;
        let qualified = qualified_key(namespace, name);
        let blob_name = safe_blob_name(&qualified);
        let blob = encrypt_with_key(&key, value.as_bytes(), qualified.as_bytes())?;
        let dir = self
            .path
            .parent()
            .ok_or_else(|| SecretError::storage("vault path has no parent directory"))?;
        std::fs::create_dir_all(dir).map_err(|error| SecretError::storage(error.to_string()))?;
        write_blob(dir, &blob_name, &blob)?;
        let mut index = self.index.lock().unwrap();
        index.entries.insert(qualified, Some(blob_name));
        save_index(&self.path, &index)
    }

    fn get(&self, namespace: &str, name: &str) -> Result<String, SecretError> {
        self.ensure_loaded()?;
        let key = self
            .key
            .read()
            .unwrap()
            .clone()
            .ok_or(SecretError::FallbackLocked)?;
        let index = self.index.lock().unwrap();
        let qualified = qualified_key(namespace, name);
        let blob_name = index
            .entries
            .get(&qualified)
            .and_then(Option::as_ref)
            .ok_or_else(|| SecretError::NotFound {
                namespace: namespace.to_string(),
                name: name.to_string(),
            })?;
        let dir = self
            .path
            .parent()
            .ok_or_else(|| SecretError::storage("vault path has no parent directory"))?;
        let blob = read_blob(dir, blob_name)?;
        let plaintext = decrypt_with_key(&key, &blob, qualified.as_bytes())?;
        String::from_utf8(plaintext).map_err(|error| SecretError::storage(error.to_string()))
    }

    fn delete(&self, namespace: &str, name: &str) -> Result<(), SecretError> {
        self.ensure_loaded()?;
        let mut index = self.index.lock().unwrap();
        if let Some(Some(blob_name)) = index.entries.remove(&qualified_key(namespace, name)) {
            let dir = self
                .path
                .parent()
                .ok_or_else(|| SecretError::storage("vault path has no parent directory"))?;
            let blob_path = dir.join(blob_name);
            let _ = std::fs::remove_file(blob_path);
        }
        save_index(&self.path, &index)
    }

    fn list(&self, namespace: Option<&str>) -> Result<Vec<SecretEntry>, SecretError> {
        self.ensure_loaded()?;
        let index = self.index.lock().unwrap();
        Ok(index
            .entries
            .keys()
            .filter_map(|qualified| parse_qualified(qualified))
            .filter(|entry| namespace.is_none_or(|wanted| wanted == entry.namespace))
            .collect())
    }
}

fn qualified_key(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

fn parse_qualified(qualified: &str) -> Option<SecretEntry> {
    let (namespace, name) = qualified.split_once('/')?;
    Some(SecretEntry {
        namespace: namespace.to_string(),
        name: name.to_string(),
    })
}

fn derive_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, SecretError> {
    let params = Params::new(19 * 1024, 2, 1, Some(32))
        .map_err(|error| SecretError::storage(error.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut output = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(password.as_bytes(), salt, output.as_mut())
        .map_err(|error| SecretError::storage(error.to_string()))?;
    Ok(output)
}

fn encrypt_with_key(
    key: &Zeroizing<[u8; 32]>,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<EncryptedBlob, SecretError> {
    let nonce = random_bytes(NONCE_LEN);
    let cipher = ChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|error| SecretError::storage(error.to_string()))?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|error| SecretError::storage(error.to_string()))?;
    Ok(EncryptedBlob {
        version: VAULT_VERSION,
        nonce: encode_blob_part(&nonce),
        ciphertext: encode_blob_part(&ciphertext),
    })
}

fn decrypt_with_key(
    key: &Zeroizing<[u8; 32]>,
    blob: &EncryptedBlob,
    aad: &[u8],
) -> Result<Vec<u8>, SecretError> {
    if blob.version != VAULT_VERSION {
        return Err(SecretError::storage(format!(
            "unsupported encrypted secret version {}",
            blob.version
        )));
    }
    let nonce = decode_blob_part(&blob.nonce)?;
    if nonce.len() != NONCE_LEN {
        return Err(SecretError::storage(
            "encrypted secret has an invalid nonce",
        ));
    }
    let ciphertext = decode_blob_part(&blob.ciphertext)?;
    let cipher = ChaCha20Poly1305::new_from_slice(key.as_ref())
        .map_err(|error| SecretError::storage(error.to_string()))?;
    cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: ciphertext.as_ref(),
                aad,
            },
        )
        .map_err(|_| SecretError::InvalidMasterPassword)
}

fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

fn encode_blob_part(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_blob_part(encoded: &str) -> Result<Vec<u8>, SecretError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| SecretError::storage(error.to_string()))
}

fn load_index(path: &Path) -> Result<VaultIndex, SecretError> {
    if !path.exists() {
        return Err(SecretError::NotFound {
            namespace: String::new(),
            name: String::new(),
        });
    }
    let text =
        std::fs::read_to_string(path).map_err(|error| SecretError::storage(error.to_string()))?;
    let index: VaultIndex =
        serde_json::from_str(&text).map_err(|error| SecretError::storage(error.to_string()))?;
    if index.version != VAULT_VERSION {
        return Err(SecretError::storage(format!(
            "unsupported secret vault version {}",
            index.version
        )));
    }
    Ok(index)
}

fn save_index(path: &Path, index: &VaultIndex) -> Result<(), SecretError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| SecretError::storage(error.to_string()))?;
    }
    let text = serde_json::to_string_pretty(index)
        .map_err(|error| SecretError::storage(error.to_string()))?;
    secure_atomic_write(path, text.as_bytes())
}

fn write_blob(dir: &Path, name: &str, blob: &EncryptedBlob) -> Result<(), SecretError> {
    let text =
        serde_json::to_string(blob).map_err(|error| SecretError::storage(error.to_string()))?;
    secure_atomic_write(&dir.join(name), text.as_bytes())
}

fn empty_index() -> VaultIndex {
    VaultIndex {
        version: VAULT_VERSION,
        salt: None,
        entries: BTreeMap::new(),
        verifier: None,
    }
}

fn safe_blob_name(qualified: &str) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(qualified.as_bytes());
    format!("{encoded}.enc")
}

fn secure_atomic_write(path: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| SecretError::storage(error.to_string()))?;
    }
    if !path.exists() {
        return create_private_file(path, bytes);
    }
    ensure_private_permissions(path)?;
    helix_fs::write_atomic(path, bytes).map_err(|error| SecretError::storage(error.to_string()))
}

#[cfg(unix)]
fn ensure_private_permissions(path: &Path) -> Result<(), SecretError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| SecretError::storage(error.to_string()))
}

#[cfg(not(unix))]
fn ensure_private_permissions(_path: &Path) -> Result<(), SecretError> {
    Ok(())
}

#[cfg(unix)]
fn create_private_file(path: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| SecretError::storage(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| SecretError::storage(error.to_string()))
}

#[cfg(not(unix))]
fn create_private_file(path: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| SecretError::storage(error.to_string()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| SecretError::storage(error.to_string()))
}

fn read_blob(dir: &Path, name: &str) -> Result<EncryptedBlob, SecretError> {
    let text = std::fs::read_to_string(dir.join(name))
        .map_err(|error| SecretError::storage(error.to_string()))?;
    serde_json::from_str(&text).map_err(|error| SecretError::storage(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_store_requires_unlock() {
        let dir = std::env::temp_dir().join(format!("helix-fallback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let backend = EncryptedFileBackend::new(dir.join("vault.json"));
        assert!(matches!(
            backend.store("helix", "x", "secret"),
            Err(SecretError::FallbackLocked)
        ));
    }

    #[test]
    fn fallback_round_trips_after_unlock() {
        let dir = std::env::temp_dir().join(format!("helix-fallback-2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let backend = EncryptedFileBackend::new(dir.join("vault.json"));
        backend.unlock("master-password").unwrap();
        backend.store("helix", "openai", "sk-live").unwrap();
        assert_eq!(backend.get("helix", "openai").unwrap(), "sk-live");
        let listed = backend.list(Some("helix")).unwrap();
        assert_eq!(listed.len(), 1);
        backend.delete("helix", "openai").unwrap();
        assert!(backend.get("helix", "openai").is_err());
    }

    #[test]
    fn master_password_is_verified_even_when_the_vault_has_no_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.json");
        EncryptedFileBackend::new(path.clone())
            .unlock("correct-master-password")
            .unwrap();

        let reopened = EncryptedFileBackend::new(path);
        assert!(matches!(
            reopened.unlock("wrong-master-password"),
            Err(SecretError::InvalidMasterPassword)
        ));
    }

    #[test]
    fn keychain_metadata_is_listable_without_becoming_a_fake_encrypted_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.json");
        let backend = EncryptedFileBackend::new(path.clone());
        backend.record_index("helix", "openai.work").unwrap();

        let reopened = EncryptedFileBackend::new(path);
        assert_eq!(
            reopened.list(Some("helix")).unwrap(),
            vec![SecretEntry {
                namespace: "helix".into(),
                name: "openai.work".into(),
            }]
        );
        reopened.unlock("master-password").unwrap();
    }

    #[test]
    fn corrupt_vault_is_reported_instead_of_silently_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.json");
        std::fs::write(&path, "{not-json").unwrap();
        let backend = EncryptedFileBackend::new(path);
        assert!(matches!(backend.list(None), Err(SecretError::Storage(_))));
        assert!(matches!(
            backend.unlock("master-password"),
            Err(SecretError::Storage(_))
        ));
    }

    #[test]
    fn encrypted_files_do_not_contain_plaintext_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.json");
        let backend = EncryptedFileBackend::new(path);
        backend.unlock("master-password").unwrap();
        backend
            .store("helix", "provider", "plaintext-must-not-survive")
            .unwrap();

        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(
                !bytes
                    .windows(b"plaintext-must-not-survive".len())
                    .any(|window| window == b"plaintext-must-not-survive")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn encrypted_vault_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.json");
        let backend = EncryptedFileBackend::new(path);
        backend.unlock("master-password").unwrap();
        backend.store("helix", "provider", "private-value").unwrap();

        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let mode = entry.unwrap().metadata().unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
