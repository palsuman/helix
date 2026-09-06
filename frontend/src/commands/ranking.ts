import type { CommandDescriptor } from "../generated/CommandDescriptor";
import { evaluateEnablement, type CommandContext } from "./context";

export interface RankedCommand {
  command: CommandDescriptor;
  score: number;
  enabled: boolean;
  disabledReason: string | null;
}

export interface CommandGroup {
  category: string;
  commands: RankedCommand[];
}

export function fuzzyScore(candidate: string, query: string): number | null {
  const text = candidate.toLocaleLowerCase();
  const needle = query.trim().toLocaleLowerCase();
  if (needle.length === 0) return 0;
  if (text === needle) return 10_000;

  let score = text.startsWith(needle) ? 2_000 : 0;
  let textIndex = 0;
  let previousMatch = -2;
  for (const character of needle) {
    const match = text.indexOf(character, textIndex);
    if (match < 0) return null;
    const boundary = match === 0 || /[\s:._/-]/.test(text[match - 1] ?? "");
    score += boundary ? 120 : 20;
    if (match === previousMatch + 1) score += 45;
    score -= Math.max(0, match - textIndex) * 2;
    previousMatch = match;
    textIndex = match + 1;
  }
  return score - Math.max(0, text.length - needle.length);
}

export function rankCommands(
  commands: readonly CommandDescriptor[],
  query: string,
  recentIds: readonly string[],
  context: CommandContext,
): RankedCommand[] {
  const recent = new Map(recentIds.map((id, index) => [id, index]));
  return commands
    .flatMap((command) => {
      const score = fuzzyScore(`${command.title} ${command.category}`, query);
      if (score === null) return [];
      const enabled = evaluateEnablement(command.enablement, context);
      return [
        {
          command,
          score,
          enabled,
          disabledReason: enabled
            ? null
            : (command.disabled_reason ?? "Command unavailable in the current context."),
        } satisfies RankedCommand,
      ];
    })
    .sort((left, right) => {
      if (query.trim() !== "" && right.score !== left.score) return right.score - left.score;
      const leftRecent = recent.get(left.command.id) ?? Number.MAX_SAFE_INTEGER;
      const rightRecent = recent.get(right.command.id) ?? Number.MAX_SAFE_INTEGER;
      if (leftRecent !== rightRecent) return leftRecent - rightRecent;
      return left.command.title.localeCompare(right.command.title);
    });
}

export function groupCommands(commands: readonly RankedCommand[]): CommandGroup[] {
  const groups = new Map<string, RankedCommand[]>();
  for (const command of commands) {
    const group = groups.get(command.command.category) ?? [];
    group.push(command);
    groups.set(command.command.category, group);
  }
  return [...groups].map(([category, groupedCommands]) => ({
    category,
    commands: groupedCommands,
  }));
}
