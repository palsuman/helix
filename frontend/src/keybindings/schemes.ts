import type { KeybindingPlatform, KeybindingRule, ResolvedBinding } from "./resolver";

export const SCHEMES = ["VS Code", "JetBrains", "Vim (basic motions)", "Emacs (basic)"] as const;
export type KeybindingScheme = (typeof SCHEMES)[number];

export function detectPlatform(platform = navigator.platform): KeybindingPlatform {
  if (/mac/i.test(platform)) return "mac";
  return /win/i.test(platform) ? "windows" : "linux";
}

export function defaultBindings(platform: KeybindingPlatform): KeybindingRule[] {
  const modifier = platform === "mac" ? "meta" : "ctrl";
  return [
    { key: `${modifier}+shift+p`, command: "workbench.action.showCommands" },
    { key: `${modifier}+p`, command: "workbench.action.quickOpen" },
    { key: "f1", command: "workbench.action.showCommands" },
    { key: `${modifier}+k ${modifier}+s`, command: "workbench.action.openGlobalKeybindings" },
    { key: `${modifier}+k z`, command: "workbench.action.toggleZenMode" },
    { key: `${modifier}+shift+n`, command: "workbench.action.newWindow" },
    { key: `${modifier}+shift+w`, command: "workbench.action.closeWindow" },
    { key: `${modifier}+j`, command: "workbench.action.togglePanel", when: "!inputFocus" },
    {
      key: platform === "linux" ? "ctrl+shift+i" : "shift+alt+f",
      command: "editor.action.formatDocument",
      when: "editorTextFocus && !editorReadonly",
    },
  ];
}

export function schemeBindings(
  scheme: KeybindingScheme,
  platform: KeybindingPlatform,
): KeybindingRule[] {
  const modifier = platform === "mac" ? "meta" : "ctrl";
  const defaults = defaultBindings(platform);
  if (scheme === "VS Code") return defaults;
  if (scheme === "JetBrains")
    return [
      ...defaults.filter(
        (rule) =>
          ![
            "workbench.action.showCommands",
            "editor.action.formatDocument",
            "workbench.action.openGlobalKeybindings",
          ].includes(rule.command),
      ),
      { key: `${modifier}+shift+a`, command: "workbench.action.showCommands" },
      {
        key: `${modifier}+alt+l`,
        command: "editor.action.formatDocument",
        when: "editorTextFocus && !editorReadonly",
      },
      { key: `${modifier}+alt+s`, command: "workbench.action.openGlobalKeybindings" },
    ];
  const motions =
    scheme === "Vim (basic motions)"
      ? [
          ["h", "cursorLeft"],
          ["j", "cursorDown"],
          ["k", "cursorUp"],
          ["l", "cursorRight"],
          ["0", "cursorHome"],
          ["shift+4", "cursorEnd"],
          ["g g", "cursorTop"],
          ["shift+g", "cursorBottom"],
        ]
      : [
          ["ctrl+b", "cursorLeft"],
          ["ctrl+n", "cursorDown"],
          ["ctrl+p", "cursorUp"],
          ["ctrl+f", "cursorRight"],
          ["ctrl+a", "cursorHome"],
          ["ctrl+e", "cursorEnd"],
          ["alt+v", "cursorPageUp"],
          ["ctrl+v", "cursorPageDown"],
        ];
  return [
    ...defaults,
    ...motions.map(([key, command]) => ({
      key,
      command,
      when:
        scheme === "Vim (basic motions)"
          ? "editorTextFocus && vimMode == 'normal'"
          : "editorTextFocus",
    })),
  ];
}

export function importScheme(
  user: readonly KeybindingRule[],
  current: readonly ResolvedBinding[],
  scheme: KeybindingScheme,
  platform: KeybindingPlatform,
): KeybindingRule[] {
  const imported = schemeBindings(scheme, platform);
  const commands = new Set(imported.map((rule) => rule.command));
  return [
    ...user.filter((rule) => !commands.has(rule.command)),
    ...current
      .filter((rule) => rule.source !== "user" && commands.has(rule.command))
      .map((rule) => ({
        key: rule.key,
        command: `-${rule.command}`,
        ...(rule.when ? { when: rule.when } : {}),
      })),
    ...imported,
  ];
}

export function displayShortcut(key: string, platform: KeybindingPlatform): string {
  const names: Record<string, string> = {
    meta: platform === "mac" ? "Cmd" : "Win",
    ctrl: "Ctrl",
    alt: platform === "mac" ? "Option" : "Alt",
    shift: "Shift",
    plus: "+",
    space: "Space",
  };
  return key
    .split(" ")
    .map((chord) =>
      chord
        .split("+")
        .map(
          (part) =>
            names[part] ??
            (part.length === 1 ? part.toUpperCase() : part[0].toUpperCase() + part.slice(1)),
        )
        .join("+"),
    )
    .join(" ");
}
