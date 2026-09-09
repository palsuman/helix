//! Runtime command catalog and IPC boundary (Task 2.8, REQ-WB-002).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use helix_core::error::AppError;
use helix_ipc::IpcDispatcher;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

pub const LIST: &str = "command.list";
pub const REGISTER: &str = "command.register";
pub const UNREGISTER: &str = "command.unregister";
pub const EXECUTE: &str = "command.execute";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandTarget {
    Renderer,
    Ipc { command: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CommandDescriptor {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title_message_id: Option<String>,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub category_message_id: Option<String>,
    pub enablement: Option<String>,
    pub disabled_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub disabled_reason_message_id: Option<String>,
    pub keybinding: Option<String>,
    pub source: String,
    pub target: CommandTarget,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CommandListRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CommandListResponse {
    pub commands: Vec<CommandDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CommandRegisterRequest {
    pub command: CommandDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CommandRegisterResponse {
    pub replaced: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CommandUnregisterRequest {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CommandUnregisterResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
#[serde(default)]
pub struct CommandExecuteRequest {
    pub id: String,
    #[ts(type = "unknown")]
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct CommandExecuteResponse {
    pub id: String,
    pub target: CommandTarget,
    #[ts(type = "unknown")]
    pub arguments: Value,
}

#[derive(Default)]
pub struct CommandRegistry {
    commands: RwLock<HashMap<String, CommandDescriptor>>,
}

impl CommandRegistry {
    pub fn with_builtins() -> Self {
        let registry = Self::default();
        for command in builtin_commands() {
            registry
                .register(command)
                .expect("built-in command must be valid");
        }
        registry
    }

    pub fn register(&self, command: CommandDescriptor) -> Result<bool, AppError> {
        validate_descriptor(&command)?;
        Ok(self
            .commands
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .insert(command.id.clone(), command)
            .is_some())
    }

    pub fn unregister(&self, id: &str) -> bool {
        self.commands
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(id)
            .is_some()
    }

    pub fn list(&self) -> Vec<CommandDescriptor> {
        let mut commands: Vec<_> = self
            .commands
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect();
        commands.sort_by(|left, right| {
            left.category
                .cmp(&right.category)
                .then_with(|| left.title.cmp(&right.title))
                .then_with(|| left.id.cmp(&right.id))
        });
        commands
    }

    pub fn resolve(&self, id: &str, arguments: Value) -> Result<CommandExecuteResponse, AppError> {
        let command = self
            .commands
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .cloned()
            .ok_or_else(|| {
                AppError::permanent(
                    "UNKNOWN_COMMAND",
                    format!("no command registered with id '{id}'"),
                )
            })?;
        Ok(CommandExecuteResponse {
            id: command.id,
            target: command.target,
            arguments,
        })
    }
}

fn validate_descriptor(command: &CommandDescriptor) -> Result<(), AppError> {
    if command.id.trim().is_empty()
        || command.title.trim().is_empty()
        || command.category.trim().is_empty()
        || command.source.trim().is_empty()
    {
        return Err(AppError::permanent(
            "INVALID_COMMAND",
            "command id, title, category, and source must not be empty",
        ));
    }
    if matches!(&command.target, CommandTarget::Ipc { command } if command == EXECUTE) {
        return Err(AppError::permanent(
            "INVALID_COMMAND_TARGET",
            "a command cannot target command.execute recursively",
        ));
    }
    Ok(())
}

fn renderer(
    id: &str,
    title: &str,
    category: &str,
    enablement: Option<&str>,
    disabled_reason: Option<&str>,
    keybinding: Option<&str>,
) -> CommandDescriptor {
    CommandDescriptor {
        id: id.into(),
        title: title.into(),
        title_message_id: Some(format!("command.{id}.title")),
        category: category.into(),
        category_message_id: Some(format!("command.category.{}", category.to_lowercase())),
        enablement: enablement.map(str::to_string),
        disabled_reason: disabled_reason.map(str::to_string),
        disabled_reason_message_id: disabled_reason.map(|_| format!("command.{id}.disabled")),
        keybinding: keybinding.map(str::to_string),
        source: "Helix".into(),
        target: CommandTarget::Renderer,
    }
}

pub fn builtin_commands() -> Vec<CommandDescriptor> {
    let mut commands = vec![
        renderer("workbench.action.revealInExplorer", "Reveal in Explorer", "View", None, None, None),
        renderer(
            "workbench.action.quickOpen",
            "Go to File",
            "View",
            None,
            None,
            Some("Ctrl+P"),
        ),
        renderer(
            "workbench.action.showCommands",
            "Show All Commands",
            "View",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.action.openGlobalKeybindings",
            "Keyboard Shortcuts",
            "Preferences",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.action.togglePanel",
            "Toggle Panel",
            "View",
            None,
            None,
            None,
        ),
        renderer(
            "editor.action.formatDocument",
            "Format Document",
            "Editor",
            Some("editorTextFocus"),
            Some("Open a text editor to format a document."),
            Some("Shift+Alt+F"),
        ),
        renderer(
            "workbench.action.toggleZenMode",
            "Toggle Zen Mode",
            "View",
            None,
            None,
            Some("Ctrl+K Z"),
        ),
        renderer(
            "workbench.layoutProfile.save",
            "Save Layout Profile",
            "View",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.layoutProfile.switch",
            "Switch Layout Profile",
            "View",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.layoutProfile.rename",
            "Rename Layout Profile",
            "View",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.layoutProfile.delete",
            "Delete Layout Profile",
            "View",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.layoutProfile.list",
            "List Layout Profiles",
            "View",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.action.newWindow",
            "New Window",
            "Window",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.action.openFolderInNewWindow",
            "Open Folder in New Window",
            "Window",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.action.duplicateWorkspaceInNewWindow",
            "Duplicate Workspace in New Window",
            "Window",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.action.closeWindow",
            "Close Window",
            "Window",
            None,
            None,
            None,
        ),
        renderer(
            "workbench.action.moveEditorToNewWindow",
            "Move Editor to New Window",
            "Window",
            Some("activeEditor"),
            Some("Open an editor before moving it to a new window."),
            None,
        ),
    ];
    for (id, title) in [
        ("cursorLeft", "Move Cursor Left"),
        ("cursorRight", "Move Cursor Right"),
        ("cursorUp", "Move Cursor Up"),
        ("cursorDown", "Move Cursor Down"),
        ("cursorHome", "Move to Line Start"),
        ("cursorEnd", "Move to Line End"),
        ("cursorTop", "Move to Document Start"),
        ("cursorBottom", "Move to Document End"),
        ("cursorPageUp", "Move Cursor Page Up"),
        ("cursorPageDown", "Move Cursor Page Down"),
    ] {
        commands.push(renderer(
            id,
            title,
            "Editor",
            Some("editorTextFocus"),
            Some("Focus a text editor to move the cursor."),
            None,
        ));
    }
    commands
}

pub fn register_commands(dispatcher: &mut IpcDispatcher, registry: Arc<CommandRegistry>) {
    let list_registry = registry.clone();
    dispatcher.register(LIST, move |_request: CommandListRequest, _ctx| {
        let registry = list_registry.clone();
        async move {
            Ok::<_, AppError>(CommandListResponse {
                commands: registry.list(),
            })
        }
    });

    let register_registry = registry.clone();
    dispatcher.register(REGISTER, move |request: CommandRegisterRequest, _ctx| {
        let registry = register_registry.clone();
        async move {
            Ok::<_, AppError>(CommandRegisterResponse {
                replaced: registry.register(request.command)?,
            })
        }
    });

    let unregister_registry = registry.clone();
    dispatcher.register(
        UNREGISTER,
        move |request: CommandUnregisterRequest, _ctx| {
            let registry = unregister_registry.clone();
            async move {
                Ok::<_, AppError>(CommandUnregisterResponse {
                    removed: registry.unregister(&request.id),
                })
            }
        },
    );

    dispatcher.register(EXECUTE, move |request: CommandExecuteRequest, _ctx| {
        let registry = registry.clone();
        async move { registry.resolve(&request.id, request.arguments) }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_ipc::IpcRequest;
    use serde_json::json;

    fn plugin_command(id: &str, title: &str) -> CommandDescriptor {
        CommandDescriptor {
            id: id.into(),
            title: title.into(),
            title_message_id: None,
            category: "Plugin".into(),
            category_message_id: None,
            enablement: None,
            disabled_reason: None,
            disabled_reason_message_id: None,
            keybinding: None,
            source: "example.plugin".into(),
            target: CommandTarget::Ipc {
                command: "plugin.execute".into(),
            },
        }
    }

    #[test]
    fn dynamic_registration_replaces_and_sorts_commands() {
        let registry = CommandRegistry::default();
        assert!(
            !registry
                .register(plugin_command("plugin.z", "Zulu"))
                .unwrap()
        );
        assert!(
            !registry
                .register(plugin_command("plugin.a", "Alpha"))
                .unwrap()
        );
        assert!(
            registry
                .register(plugin_command("plugin.a", "Alpha 2"))
                .unwrap()
        );
        assert_eq!(
            registry
                .list()
                .iter()
                .map(|command| command.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha 2", "Zulu"]
        );
        assert!(registry.unregister("plugin.z"));
        assert!(!registry.unregister("plugin.missing"));
    }

    #[tokio::test]
    async fn ipc_commands_list_register_and_resolve_targets() {
        let registry = Arc::new(CommandRegistry::default());
        let mut dispatcher = IpcDispatcher::new();
        register_commands(&mut dispatcher, registry);

        let register = dispatcher
            .dispatch(IpcRequest::new(
                REGISTER,
                "register-1",
                serde_json::to_value(CommandRegisterRequest {
                    command: plugin_command("plugin.format", "Format Special"),
                })
                .unwrap(),
            ))
            .await;
        assert_eq!(register.result.unwrap()["replaced"], false);

        let execute = dispatcher
            .dispatch(IpcRequest::new(
                EXECUTE,
                "execute-1",
                json!({ "id": "plugin.format", "arguments": { "mode": "fast" } }),
            ))
            .await;
        assert_eq!(execute.result.unwrap()["target"]["kind"], "ipc");

        let unknown = dispatcher
            .dispatch(IpcRequest::new(
                EXECUTE,
                "execute-2",
                json!({ "id": "missing", "arguments": null }),
            ))
            .await;
        assert_eq!(unknown.error.unwrap().code, "UNKNOWN_COMMAND");
    }
}
