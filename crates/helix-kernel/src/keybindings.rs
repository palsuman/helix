use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use helix_core::error::AppError;
use helix_ipc::IpcDispatcher;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const GET: &str = "keybindings.get";
pub const SET: &str = "keybindings.set";
pub const CONTRIBUTE: &str = "keybindings.contribute";
pub const REMOVE_CONTRIBUTION: &str = "keybindings.removeContribution";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct KeybindingRule {
    pub key: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub args: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mac: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub win: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub linux: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct KeybindingContribution {
    pub owner: String,
    pub rules: Vec<KeybindingRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct KeybindingsSnapshot {
    pub user: Vec<KeybindingRule>,
    pub plugins: Vec<KeybindingContribution>,
    pub warnings: Vec<String>,
    pub revision: String,
    pub writable: bool,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct KeybindingsSetRequest {
    pub rules: Vec<KeybindingRule>,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct KeybindingOwnerRequest {
    pub owner: String,
}

#[derive(Default)]
struct BindingState {
    user: Vec<KeybindingRule>,
    plugins: Vec<KeybindingContribution>,
}

pub struct KeybindingService {
    path: Option<PathBuf>,
    state: Mutex<BindingState>,
}

fn revision(bytes: &[u8]) -> String {
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    format!("{:016x}", hash.finish())
}

fn valid_key(key: &str) -> bool {
    let chords: Vec<_> = key.split_whitespace().collect();
    !chords.is_empty()
        && chords.len() <= 4
        && chords.iter().all(|chord| {
            let normalized = chord.to_lowercase().replace("++", "+plus");
            let parts: Vec<_> = normalized.split('+').collect();
            let Some((key, modifiers)) = parts.split_last() else {
                return false;
            };
            let named = [
                "space",
                "enter",
                "return",
                "escape",
                "esc",
                "tab",
                "backspace",
                "delete",
                "del",
                "insert",
                "home",
                "end",
                "pageup",
                "pagedown",
                "left",
                "right",
                "up",
                "down",
                "arrowleft",
                "arrowright",
                "arrowup",
                "arrowdown",
                "plus",
                "minus",
            ];
            let function_key = key
                .strip_prefix('f')
                .and_then(|value| value.parse::<u8>().ok())
                .is_some_and(|value| (1..=24).contains(&value));
            (key.chars().count() == 1 || named.contains(key) || function_key)
                && modifiers.iter().all(|modifier| {
                    [
                        "ctrl", "control", "alt", "option", "shift", "meta", "cmd", "command",
                        "win", "super",
                    ]
                    .contains(modifier)
                })
        })
}

fn valid_rule(rule: &KeybindingRule) -> bool {
    !rule.command.trim().is_empty()
        && rule.command != "-"
        && rule.command.len() <= 256
        && valid_key(&rule.key)
        && [&rule.mac, &rule.win, &rule.linux]
            .into_iter()
            .all(|key| key.as_deref().is_none_or(valid_key))
        && rule.when.as_ref().is_none_or(|when| when.len() <= 4096)
}

impl KeybindingService {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            state: Mutex::new(BindingState::default()),
        }
    }

    pub fn for_user() -> Self {
        Self::new(
            helix_config::ConfigPaths::for_user()
                .user
                .map(|path| path.with_file_name("keybindings.json")),
        )
    }

    fn refresh(&self, state: &mut BindingState) -> KeybindingsSnapshot {
        let mut snapshot = KeybindingsSnapshot {
            user: state.user.clone(),
            plugins: state.plugins.clone(),
            warnings: vec![],
            revision: "unavailable".into(),
            writable: false,
            path: self
                .path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        };
        let Some(path) = &self.path else {
            snapshot
                .warnings
                .push("The user configuration directory is unavailable.".into());
            return snapshot;
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                state.user.clear();
                snapshot.user.clear();
                snapshot.revision = "missing".into();
                snapshot.writable = true;
                return snapshot;
            }
            Err(error) => {
                snapshot
                    .warnings
                    .push(format!("Cannot read keybindings: {error}"));
                return snapshot;
            }
        };
        snapshot.revision = revision(&bytes);
        let parsed = std::str::from_utf8(&bytes)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                serde_json::from_str::<Vec<Value>>(&helix_config::jsonc::blank_comments(text))
                    .map_err(|error| error.to_string())
            });
        match parsed {
            Ok(entries) => {
                let mut rules = Vec::new();
                for (index, value) in entries.into_iter().enumerate() {
                    match serde_json::from_value::<KeybindingRule>(value) {
                        Ok(rule) if valid_rule(&rule) => rules.push(rule),
                        _ => snapshot.warnings.push(format!(
                            "Skipped invalid keybinding at entry {}.",
                            index + 1
                        )),
                    }
                }
                state.user = rules.clone();
                snapshot.user = rules;
                snapshot.writable = true;
            }
            Err(error) => snapshot.warnings.push(format!(
                "Invalid keybindings.json: {error}. Using the last valid bindings."
            )),
        }
        snapshot
    }

    pub fn get(&self) -> KeybindingsSnapshot {
        self.refresh(&mut self.state.lock().unwrap_or_else(|error| error.into_inner()))
    }

    pub fn set(&self, request: KeybindingsSetRequest) -> Result<KeybindingsSnapshot, AppError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let current = self.refresh(&mut state);
        if !current.writable {
            return Err(AppError::permanent(
                "KEYBINDINGS_READ_ONLY",
                "Repair the keybindings file or user configuration directory before saving.",
            ));
        }
        if current.revision != request.revision {
            return Err(AppError::transient(
                "KEYBINDINGS_CONFLICT",
                "Keybindings changed in another window or on disk. Reload before saving.",
            ));
        }
        if request.rules.iter().any(|rule| !valid_rule(rule)) {
            return Err(AppError::permanent(
                "INVALID_KEYBINDING",
                "One or more keybindings are invalid.",
            ));
        }
        let path = self.path.as_ref().unwrap();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                AppError::transient("KEYBINDINGS_WRITE_FAILED", error.to_string())
            })?;
        }
        let bytes = serde_json::to_vec_pretty(&request.rules)
            .map_err(|error| AppError::permanent("INVALID_KEYBINDING", error.to_string()))?;
        helix_fs::atomic::write_atomic(path, &bytes)
            .map_err(|error| AppError::transient("KEYBINDINGS_WRITE_FAILED", error.to_string()))?;
        Ok(self.refresh(&mut state))
    }

    pub fn contribute(
        &self,
        contribution: KeybindingContribution,
    ) -> Result<KeybindingsSnapshot, AppError> {
        if contribution.owner.trim().is_empty()
            || contribution
                .rules
                .iter()
                .any(|rule| !valid_rule(rule) || rule.command.starts_with('-'))
        {
            return Err(AppError::permanent(
                "INVALID_KEYBINDING",
                "Plugin contributions require an owner and valid non-removal rules.",
            ));
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state
            .plugins
            .retain(|existing| existing.owner != contribution.owner);
        state.plugins.push(contribution);
        Ok(self.refresh(&mut state))
    }

    pub fn remove_contribution(&self, owner: &str) -> KeybindingsSnapshot {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.plugins.retain(|existing| existing.owner != owner);
        self.refresh(&mut state)
    }
}

pub fn register_commands(dispatcher: &mut IpcDispatcher, service: Arc<KeybindingService>) {
    let reader = service.clone();
    dispatcher.register(GET, move |_request: Value, _ctx| {
        let service = reader.clone();
        async move {
            tokio::task::spawn_blocking(move || service.get())
                .await
                .map_err(|error| AppError::transient("KEYBINDINGS_TASK_FAILED", error.to_string()))
        }
    });
    let writer = service.clone();
    dispatcher.register(SET, move |request: KeybindingsSetRequest, _ctx| {
        let service = writer.clone();
        async move {
            tokio::task::spawn_blocking(move || service.set(request))
                .await
                .map_err(|error| {
                    AppError::transient("KEYBINDINGS_TASK_FAILED", error.to_string())
                })?
        }
    });
    let plugins = service.clone();
    dispatcher.register(CONTRIBUTE, move |request: KeybindingContribution, _ctx| {
        let service = plugins.clone();
        async move { service.contribute(request) }
    });
    dispatcher.register(
        REMOVE_CONTRIBUTION,
        move |request: KeybindingOwnerRequest, _ctx| {
            let service = service.clone();
            async move { Ok::<_, AppError>(service.remove_contribution(&request.owner)) }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_fs::testutil::TempDir;
    use helix_ipc::IpcRequest;
    use serde_json::json;

    fn rule(command: &str) -> KeybindingRule {
        serde_json::from_value(
            json!({"key": "ctrl+k ctrl+c", "command": command, "when": "editorTextFocus"}),
        )
        .unwrap()
    }

    #[test]
    fn round_trip_user_additions_and_removals_and_reject_stale_writes() {
        let temp = TempDir::new("keybindings");
        let path = temp.path().join("keybindings.json");
        let service = KeybindingService::new(Some(path.clone()));
        let initial = service.get();
        let saved = service
            .set(KeybindingsSetRequest {
                revision: initial.revision.clone(),
                rules: vec![rule("editor.comment"), rule("-editor.old")],
            })
            .unwrap();
        assert_eq!(saved.user.len(), 2);
        assert_eq!(KeybindingService::new(Some(path)).get().user, saved.user);
        assert_eq!(
            service
                .set(KeybindingsSetRequest {
                    revision: initial.revision,
                    rules: vec![]
                })
                .unwrap_err()
                .code,
            "KEYBINDINGS_CONFLICT"
        );
    }

    #[test]
    fn external_edits_skip_invalid_rules_and_recover_from_invalid_json() {
        let temp = TempDir::new("keybindings-recovery");
        let path = temp.write("keybindings.json", "[// user bindings\n{\"key\":\"ctrl+k\",\"command\":\"run\"}, {\"key\":\"bad+key\",\"command\":\"bad\"},]");
        let service = KeybindingService::new(Some(path.clone()));
        let good = service.get();
        assert_eq!(good.user.len(), 1);
        assert_eq!(good.warnings.len(), 1);
        std::fs::write(&path, "[").unwrap();
        let broken = service.get();
        assert_eq!(broken.user, good.user);
        assert!(!broken.writable);
        assert_eq!(
            service
                .set(KeybindingsSetRequest {
                    revision: broken.revision,
                    rules: vec![]
                })
                .unwrap_err()
                .code,
            "KEYBINDINGS_READ_ONLY"
        );
        std::fs::write(&path, "[]").unwrap();
        assert!(service.get().user.is_empty());
        assert!(service.get().warnings.is_empty());
    }

    #[tokio::test]
    async fn plugin_contributions_are_dynamic_and_do_not_write_the_user_file() {
        let temp = TempDir::new("keybindings-plugin");
        let path = temp.path().join("keybindings.json");
        let service = Arc::new(KeybindingService::new(Some(path.clone())));
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, service.clone());
        let response = dispatcher
            .dispatch(IpcRequest::new(
                CONTRIBUTE,
                "plugin-1",
                json!({"owner": "example", "rules": [rule("example.run")]}),
            ))
            .await;
        assert!(response.error.is_none());
        assert_eq!(service.get().plugins.len(), 1);
        assert!(!path.exists());
        let response = dispatcher
            .dispatch(IpcRequest::new(
                REMOVE_CONTRIBUTION,
                "plugin-2",
                json!({"owner": "example"}),
            ))
            .await;
        assert!(response.error.is_none());
        assert!(service.get().plugins.is_empty());
    }
}
