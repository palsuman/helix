# Keybindings

Task 2.9 provides one window-local key resolver backed by kernel-owned user
configuration. Open **Keyboard Shortcuts** from the command palette, or press
`Ctrl+K Ctrl+S` (`Cmd+K Cmd+S` on macOS).

## User File

User bindings live in `~/.helix/keybindings.json`. The file is a JSONC array:

```jsonc
[
  {
    "key": "ctrl+alt+l",
    "mac": "meta+alt+l",
    "command": "editor.action.formatDocument",
    "when": "editorTextFocus && !editorReadonly",
  },
  {
    "key": "ctrl+shift+p",
    "command": "-workbench.action.showCommands",
  },
]
```

`mac`, `win`, and `linux` override `key` on that platform. `args` passes a JSON
value to the registered command handler. A command ID prefixed with `-` removes
matching earlier rules; an optional `when` limits the removal to that clause.
Whitespace separates chords. Modifier aliases include Ctrl/Control,
Meta/Cmd/Command, and Alt/Option. Use `space` and `plus` for those keys.

The kernel uses the configuration service's user-directory resolution and the
filesystem service's atomic writer. Saves require the revision read by the
editor, preventing stale windows from silently overwriting changes. Programmatic
writes serialize the array and do not preserve JSONC comments. Malformed JSON
retains the last good rules and disables saving until repaired. Invalid individual
rules are skipped with warnings. Each open window refreshes the shared document
and plugin contributions every second; filesystem work does not run in the WebView.

## Resolution

Rules are ordered default, plugin, user, with the last matching rule winning.
Enablement is checked separately from the binding's when-clause. A matching prefix
starts a 1.5-second chord window; Escape, blur, document visibility changes,
composition, or configuration replacement cancel it. Unmatched keys retain native
behavior. The recorder consumes its own events without executing shortcuts.

When-clauses reuse the command expression parser: boolean keys, `!`, `&&`, `||`,
parentheses, and `==`/`!=` comparisons against quoted or unquoted literals.
The conflict view includes exact-key and prefix conflicts. Except for simple
opposite clauses and constant false conditions, conditional conflicts are
conservatively shown as potential conflicts; displayed priority is the winner
when the competing clauses are simultaneously true.

## Extension Points

- `KeybindingService.contribute(owner, rules)` replaces one plugin's session
  contribution. `removeContribution(owner)` removes it without changing user data.
- `ContextKeyService.set()` supplies domain contexts such as `debugActive`,
  `workspaceOpen`, `activeEditor`, and `vimMode`. Focus tracking derives editor,
  terminal, panel, sidebar, search, notification, and shortcut-editor contexts.
- Future editor and terminal views should expose `data-keybinding-context="editor"`
  or `data-keybinding-context="terminal"`. Editor views may supply
  `data-language-id`; text inputs and textareas provide input and selection state.
- Commands continue through `CommandRegistry.execute()` and its typed IPC route.
  Register renderer handlers with `registerHandler`; unregister callbacks do not
  remove a newer replacement handler.

The VS Code and JetBrains schemes cover the commands currently available in Helix.
Vim basic motions and Emacs basic schemes bind the registered `cursorLeft`,
`cursorRight`, `cursorUp`, `cursorDown`, `cursorHome`, `cursorEnd`, `cursorTop`,
`cursorBottom`, `cursorPageUp`, and `cursorPageDown` command IDs. They are
editor-scoped; Vim motions additionally require `vimMode == normal`. These are
keymap schemes, not full emulation engines. The editor implementation must register
the cursor handlers; commands without handlers do not consume typing. The current
shell has no text-editor implementation, so formatting and cursor actions remain
unavailable there.

Importing a scheme writes user overrides for its covered commands while preserving
unrelated user bindings. Repeated imports are stable. Per-command reset removes
user additions and removals, exposing defaults and plugin rules again.
