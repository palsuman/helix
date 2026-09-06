import { evaluateEnablement, validateWhenClause, type CommandContext } from "../commands/context";
import type { KeybindingRule } from "../generated/KeybindingRule";
export type { KeybindingRule } from "../generated/KeybindingRule";

export type KeybindingPlatform = "mac" | "windows" | "linux";
export type KeybindingSource = "default" | "plugin" | "user";

export interface ResolvedBinding extends KeybindingRule {
  source: KeybindingSource;
  owner: string;
  id: string;
  chords: string[];
}

export const CHORD_TIMEOUT_MS = 1_500;
const MODIFIERS = ["ctrl", "alt", "shift", "meta"] as const;
const ALIASES: Record<string, string> = {
  control: "ctrl",
  cmd: "meta",
  command: "meta",
  win: "meta",
  super: "meta",
  option: "alt",
  esc: "escape",
  return: "enter",
  " ": "space",
  spacebar: "space",
  arrowleft: "left",
  arrowright: "right",
  arrowup: "up",
  arrowdown: "down",
  del: "delete",
  minus: "-",
};
const NAMED_KEYS = new Set([
  "space",
  "enter",
  "escape",
  "tab",
  "backspace",
  "delete",
  "insert",
  "home",
  "end",
  "pageup",
  "pagedown",
  "left",
  "right",
  "up",
  "down",
  "plus",
  "minus",
]);

export function normalizeKeybinding(value: string): string[] | null {
  if (value.trim() === "") return null;
  const result: string[] = [];
  for (const chord of value.trim().toLowerCase().split(/\s+/)) {
    const parts = chord
      .replace(/\+\+$/, "+plus")
      .split("+")
      .map((part) => ALIASES[part] ?? part);
    const key = parts.pop();
    if (!key || !(key.length === 1 || NAMED_KEYS.has(key) || /^f([1-9]|1\d|2[0-4])$/.test(key)))
      return null;
    if (
      parts.some((part) => !MODIFIERS.includes(part as (typeof MODIFIERS)[number])) ||
      new Set(parts).size !== parts.length
    )
      return null;
    result.push([...MODIFIERS.filter((modifier) => parts.includes(modifier)), key].join("+"));
  }
  return result.length <= 4 ? result : null;
}

export function eventChord(event: KeyboardEvent): string | null {
  if (
    event.isComposing ||
    event.key === "Dead" ||
    event.key === "Process" ||
    event.getModifierState("AltGraph")
  )
    return null;
  let key = event.key.toLowerCase();
  key = ALIASES[key] ?? key;
  if (MODIFIERS.includes(key as (typeof MODIFIERS)[number])) return null;
  if (event.altKey && /^Key[A-Z]$/.test(event.code)) key = event.code.slice(3).toLowerCase();
  if (event.shiftKey && /^Digit\d$/.test(event.code)) key = event.code.slice(5);
  if (key === "+") key = "plus";
  const modifiers = [
    event.ctrlKey && "ctrl",
    event.altKey && "alt",
    event.shiftKey && "shift",
    event.metaKey && "meta",
  ].filter(Boolean);
  return normalizeKeybinding([...modifiers, key].join("+"))?.[0] ?? null;
}

export function resolveBindings(
  layers: { source: KeybindingSource; owner: string; rules: readonly KeybindingRule[] }[],
  platform: KeybindingPlatform,
): { bindings: ResolvedBinding[]; warnings: string[] } {
  const bindings: ResolvedBinding[] = [];
  const warnings: string[] = [];
  const priority = { default: 0, plugin: 1, user: 2 };
  for (const layer of [...layers].sort(
    (left, right) => priority[left.source] - priority[right.source],
  )) {
    layer.rules.forEach((rule, index) => {
      const key =
        (platform === "mac" ? rule.mac : platform === "windows" ? rule.win : rule.linux) ??
        rule.key;
      const chords = typeof key === "string" ? normalizeKeybinding(key) : null;
      if (
        chords === null ||
        typeof rule.command !== "string" ||
        !rule.command.trim() ||
        rule.command === "-" ||
        !validateWhenClause(rule.when)
      ) {
        warnings.push(`${layer.owner}: skipped invalid binding ${index + 1}.`);
        return;
      }
      if (rule.command.startsWith("-")) {
        if (layer.source !== "user") return;
        for (let existing = bindings.length - 1; existing >= 0; existing -= 1) {
          const candidate = bindings[existing];
          if (
            candidate.command === rule.command.slice(1) &&
            candidate.chords.join(" ") === chords.join(" ") &&
            (rule.when === undefined || candidate.when === rule.when)
          )
            bindings.splice(existing, 1);
        }
      } else {
        bindings.push({
          ...rule,
          key: chords.join(" "),
          chords,
          source: layer.source,
          owner: layer.owner,
          id: `${layer.source}:${layer.owner}:${index}`,
        });
      }
    });
  }
  return { bindings, warnings };
}

export function findConflicts(
  bindings: readonly ResolvedBinding[],
): Map<string, ResolvedBinding[]> {
  const conflicts = new Map<string, ResolvedBinding[]>();
  for (let first = 0; first < bindings.length; first += 1) {
    for (let second = first + 1; second < bindings.length; second += 1) {
      const left = bindings[first];
      const right = bindings[second];
      if (
        left.when &&
        right.when &&
        (left.when.trim() === `!${right.when.trim()}` ||
          right.when.trim() === `!${left.when.trim()}`)
      )
        continue;
      if (left.when?.trim() === "false" || right.when?.trim() === "false") continue;
      if (left.command === right.command && left.when === right.when) continue;
      const prefix =
        left.chords.every((chord, index) => right.chords[index] === chord) ||
        right.chords.every((chord, index) => left.chords[index] === chord);
      if (!prefix) continue;
      conflicts.set(left.id, [...(conflicts.get(left.id) ?? []), right]);
      conflicts.set(right.id, [...(conflicts.get(right.id) ?? []), left]);
    }
  }
  return conflicts;
}

export class KeybindingResolver {
  private pending: string[] = [];
  private timer: ReturnType<typeof setTimeout> | undefined;
  private readonly onPending: (chords: string[]) => void;

  constructor(onPending: (chords: string[]) => void = () => {}) {
    this.onPending = onPending;
  }

  reset(): void {
    clearTimeout(this.timer);
    this.timer = undefined;
    this.pending = [];
    this.onPending([]);
  }

  handle(
    event: KeyboardEvent,
    bindings: readonly ResolvedBinding[],
    context: CommandContext,
    available: (command: string) => boolean = () => true,
  ): { consumed: boolean; binding?: ResolvedBinding } {
    if (
      event.isComposing ||
      event.key === "Dead" ||
      event.key === "Process" ||
      event.getModifierState("AltGraph")
    ) {
      this.reset();
      return { consumed: false };
    }
    if (event.defaultPrevented) return { consumed: false };
    const chord = eventChord(event);
    if (chord === null) return { consumed: false };
    if (this.pending.length && event.repeat) return { consumed: true };
    if (this.pending.length && chord === "escape") {
      this.reset();
      return { consumed: true };
    }
    const prefix = [...this.pending, chord];
    const winner = [...bindings]
      .reverse()
      .find(
        (binding) =>
          prefix.every((part, index) => binding.chords[index] === part) &&
          evaluateEnablement(binding.when, context) &&
          available(binding.command),
      );
    if (winner === undefined) {
      const wasPending = this.pending.length > 0;
      this.reset();
      if (wasPending) return this.handle(event, bindings, context, available);
      return { consumed: false };
    }
    if (prefix.length === winner.chords.length) {
      this.reset();
      if (event.repeat && !winner.command.startsWith("cursor")) return { consumed: true };
      return { consumed: true, binding: winner };
    }
    clearTimeout(this.timer);
    this.pending = prefix;
    this.onPending(prefix);
    this.timer = setTimeout(() => this.reset(), CHORD_TIMEOUT_MS);
    return { consumed: true };
  }
}
