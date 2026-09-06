import { useEffect, useId, useMemo, useRef, useState } from "react";
import type { CommandDescriptor } from "../generated/CommandDescriptor";
import { Icon } from "../icons";
import {
  localizeCommand,
  useLocalization,
  useLocalizationSnapshot,
  useMessage,
} from "../localization";
import { notify } from "../notifications";
import type { CommandContext } from "./context";
import { groupCommands, rankCommands, type RankedCommand } from "./ranking";
import type { CommandRegistry } from "./registry";
import "./commandPalette.css";

export interface CommandPaletteProps {
  registry: CommandRegistry;
  context?: CommandContext;
  shortcutFor?: (command: string) => string | null;
}

export function CommandPalette({ registry, context = {}, shortcutFor }: CommandPaletteProps) {
  const t = useMessage();
  const localization = useLocalization();
  useLocalizationSnapshot();
  const [open, setOpen] = useState(false);
  const [originContext, setOriginContext] = useState(context);
  const [query, setQuery] = useState("");
  const [commands, setCommands] = useState<CommandDescriptor[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const listId = useId();
  const localizedCommands = commands.map((command) => localizeCommand(command, localization));
  const ranked = useMemo(
    () =>
      rankCommands(
        localizedCommands,
        query,
        registry.recent(),
        originContext,
        t("commandPaletteUnavailable"),
      ),
    [localizedCommands, originContext, query, registry, t],
  );
  const groups = useMemo(() => groupCommands(ranked), [ranked]);
  const active = ranked[activeIndex];

  useEffect(() => {
    return registry.registerHandler("workbench.action.showCommands", () => {
      if (!open) {
        previousFocus.current = document.activeElement as HTMLElement | null;
        setOriginContext(context);
        setLoading(true);
        setError(null);
        setActiveIndex(0);
        setOpen(true);
      }
    });
  }, [context, open, registry]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    void registry
      .list()
      .then(
        (nextCommands) => {
          if (active) {
            setCommands(nextCommands);
            setActiveIndex(0);
          }
        },
        (cause: unknown) => {
          if (active) setError(String(cause));
        },
      )
      .finally(() => {
        if (active) setLoading(false);
      });
    requestAnimationFrame(() => inputRef.current?.focus());
    return () => {
      active = false;
    };
  }, [open, registry]);

  const close = () => {
    setOpen(false);
    setQuery("");
    setError(null);
    requestAnimationFrame(() => previousFocus.current?.focus());
  };

  const execute = async (entry: RankedCommand | undefined) => {
    if (entry === undefined || !entry.enabled) return;
    setError(null);
    try {
      await registry.execute(entry.command.id);
      close();
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      setError(message);
      notify({ kind: "error", source: t("commandPaletteSource"), message });
    }
  };

  const onDialogKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      setActiveIndex((index) => Math.min(ranked.length - 1, index + 1));
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex((index) => Math.max(0, index - 1));
    } else if (event.key === "Home") {
      event.preventDefault();
      setActiveIndex(0);
    } else if (event.key === "End") {
      event.preventDefault();
      setActiveIndex(Math.max(0, ranked.length - 1));
    } else if (event.key === "Enter") {
      event.preventDefault();
      void execute(active);
    } else if (event.key === "Tab") {
      const focusable = dialogRef.current?.querySelectorAll<HTMLElement>(
        'input, button:not([disabled]), [tabindex]:not([tabindex="-1"])',
      );
      if (focusable === undefined || focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    }
  };

  if (!open) return null;

  return (
    <div
      className="command-palette-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) close();
      }}
    >
      <div
        ref={dialogRef}
        className="command-palette"
        role="dialog"
        aria-modal="true"
        aria-label={t("commandPaletteTitle")}
        onKeyDown={onDialogKeyDown}
      >
        <div className="command-palette-input-row">
          <span aria-hidden="true">{String.fromCharCode(62)}</span>
          <input
            ref={inputRef}
            role="combobox"
            aria-label={t("commandPaletteSearch")}
            aria-expanded="true"
            aria-controls={listId}
            aria-activedescendant={
              active === undefined ? undefined : `${listId}-option-${activeIndex}`
            }
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActiveIndex(0);
            }}
            placeholder={t("commandPalettePlaceholder")}
            autoComplete="off"
            spellCheck={false}
          />
          <button type="button" aria-label={t("commandPaletteClose")} onClick={close}>
            <Icon id="close" size="sm" label={t("commandPaletteClose")} />
          </button>
        </div>
        <div id={listId} className="command-palette-results" role="listbox">
          {loading && <p className="command-palette-state">{t("commandPaletteLoading")}</p>}
          {!loading && error !== null && (
            <p className="command-palette-state" role="alert">
              {error}
            </p>
          )}
          {!loading && error === null && ranked.length === 0 && (
            <p className="command-palette-state">{t("commandPaletteEmpty")}</p>
          )}
          {!loading &&
            error === null &&
            groups.map((group) => (
              <section key={group.category} role="group" aria-label={group.category}>
                <h2>{group.category}</h2>
                {group.commands.map((entry) => {
                  const index = ranked.indexOf(entry);
                  const selected = index === activeIndex;
                  return (
                    <div
                      id={`${listId}-option-${index}`}
                      key={entry.command.id}
                      className={`command-palette-option${selected ? " is-active" : ""}`}
                      role="option"
                      aria-selected={selected}
                      aria-disabled={!entry.enabled}
                      onMouseEnter={() => setActiveIndex(index)}
                      onMouseDown={(event) => event.preventDefault()}
                      onClick={() => void execute(entry)}
                    >
                      <span className="command-palette-title">
                        <strong dir="auto">{entry.command.title}</strong>
                        <small dir="auto">
                          {entry.enabled ? entry.command.source : entry.disabledReason}
                        </small>
                      </span>
                      {(shortcutFor ? shortcutFor(entry.command.id) : entry.command.keybinding) && (
                        <kbd>
                          {shortcutFor ? shortcutFor(entry.command.id) : entry.command.keybinding}
                        </kbd>
                      )}
                    </div>
                  );
                })}
              </section>
            ))}
        </div>
      </div>
    </div>
  );
}
