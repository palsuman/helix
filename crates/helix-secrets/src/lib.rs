//! `helix-secrets` — OS-backed credential storage (Task 1.12, REQ-SEC-002).
//!
//! Secrets live in the platform keychain when available, with an encrypted
//! on-disk fallback protected by a master password. Values are never returned
//! to the frontend after storage; the kernel resolves them internally for
//! providers and git.

pub mod backend;
pub mod commands;
pub mod error;
pub mod fallback;
pub mod git;
pub mod namespace;
pub mod service;

pub use backend::{
    BackendKind, CompositeBackend, KEYRING_SERVICE, KeyringBackend, MemoryBackend, SecretBackend,
    SecretEntry, default_vault_path,
};
pub use commands::{
    DELETE, EXISTS, LIST, STATUS, STORE, SecretBackendKind, SecretRef, SecretsDeleteRequest,
    SecretsDeleteResponse, SecretsExistsRequest, SecretsExistsResponse, SecretsListRequest,
    SecretsListResponse, SecretsStatusResponse, SecretsStoreRequest, SecretsStoreResponse,
    SecretsUnlockRequest, SecretsUnlockResponse, UNLOCK,
};
pub use error::SecretError;
pub use git::{GitCredential, handle_git_credential};
pub use namespace::{GIT_NAMESPACE, HELIX_NAMESPACE, PLUGIN_NAMESPACE_PREFIX, SecretCaller};
pub use service::{LOG_SOURCE, SecretService};
