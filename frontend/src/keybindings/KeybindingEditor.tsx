import { useEffect, useRef, useState } from "react";
import { useStore } from "zustand";
import { Icon } from "../icons";
import {
  localizeCommand,
  useLocalization,
  useLocalizationSnapshot,
  useMessage,
} from "../localization";
import { validateWhenClause } from "../commands/context";
import {
  CHORD_TIMEOUT_MS,
  eventChord,
  findConflicts,
  normalizeKeybinding,
  type ResolvedBinding,
} from "./resolver";
import { displayShortcut, SCHEMES, type KeybindingScheme } from "./schemes";
import type { KeybindingService } from "./service";
import "./keybindings.css";

interface Draft {
  command: string;
  title: string;
  key: string;
  when: string;
  args?: unknown;
}

export function KeybindingEditor({ service }: { service: KeybindingService }) {
  const t = useMessage();
  const localization = useLocalization();
  useLocalizationSnapshot();
  const state = useStore(service.store);
  const [query, setQuery] = useState("");
  const [commandFilter, setCommandFilter] = useState("");
  const [conflictsOnly, setConflictsOnly] = useState(false);
  const [scheme, setScheme] = useState<KeybindingScheme>("VS Code");
  const [draft, setDraft] = useState<Draft | null>(null);
  const [recording, setRecording] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const shortcutRef = useRef<HTMLInputElement>(null);
  const recordTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const recorded = useRef<string[]>([]);
  const conflicts = findConflicts(state.bindings);
  const writable = !!state.document?.writable && !saving;
  const titles = new Map(
    state.commands.map((command) => [command.id, localizeCommand(command, localization).title]),
  );
  const commandIds = [
    ...new Set([
      ...state.commands.map((command) => command.id),
      ...state.bindings.map((binding) => binding.command),
    ]),
  ];
  const rows = commandIds
    .flatMap((command) => {
      const bindings = state.bindings.filter((binding) => binding.command === command);
      return (bindings.length ? bindings : [null]).map((binding) => ({
        command,
        title: titles.get(command) ?? command,
        binding,
      }));
    })
    .filter((row) => {
      if (commandFilter && row.command !== commandFilter) return false;
      if (conflictsOnly && (!row.binding || !conflicts.has(row.binding.id))) return false;
      return [row.title, row.command, row.binding?.key, row.binding?.when, row.binding?.owner]
        .join(" ")
        .toLowerCase()
        .includes(query.toLowerCase());
    });

  useEffect(() => {
    searchRef.current?.focus();
    return () => clearTimeout(recordTimer.current);
  }, []);

  const perform = async (operation: () => Promise<void>, closeDraft = true) => {
    setSaving(true);
    setError(null);
    try {
      await operation();
      if (closeDraft) setDraft(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  };

  const edit = (command: string, title: string, binding: ResolvedBinding | null) => {
    clearTimeout(recordTimer.current);
    setRecording(false);
    setError(null);
    setDraft({
      command,
      title,
      key: binding?.key ?? "",
      when: binding?.when ?? "",
      args: binding?.args,
    });
    requestAnimationFrame(() => shortcutRef.current?.focus());
  };

  const record = () => {
    clearTimeout(recordTimer.current);
    recorded.current = [];
    setRecording(!recording);
    shortcutRef.current?.focus();
  };

  const close = () => service.store.setState({ editorOpen: false });
  const validDraft =
    draft !== null && normalizeKeybinding(draft.key) !== null && validateWhenClause(draft.when);
  const competing = draft
    ? state.bindings.filter(
        (binding) =>
          binding.command !== draft.command &&
          binding.key === normalizeKeybinding(draft.key)?.join(" "),
      )
    : [];

  return (
    <section
      className="keybinding-editor"
      aria-label={t("keybindingsTitle")}
      onKeyDown={(event) => {
        if (event.key === "Escape" && !recording) {
          event.preventDefault();
          if (draft) setDraft(null);
          else close();
        }
      }}
    >
      <header className="keybinding-header">
        <h1>
          <Icon id="keyboard" />
          {t("keybindingsTitle")}
        </h1>
        <button
          type="button"
          className="keybinding-icon-button"
          aria-label={t("keybindingsClose")}
          title={t("keybindingsClose")}
          onClick={close}
        >
          <Icon id="close" label={t("keybindingsClose")} />
        </button>
      </header>
      <div className="keybinding-filters">
        <input
          ref={searchRef}
          type="search"
          aria-label={t("keybindingsSearch")}
          placeholder={t("keybindingsSearch")}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <select
          aria-label={t("keybindingsFilterCommand")}
          value={commandFilter}
          onChange={(event) => setCommandFilter(event.target.value)}
        >
          <option value="">{t("keybindingsAllCommands")}</option>
          {commandIds.map((command) => (
            <option key={command} value={command}>
              {titles.get(command) ?? command}
            </option>
          ))}
        </select>
        <label className="keybinding-conflict-toggle">
          <input
            type="checkbox"
            checked={conflictsOnly}
            onChange={(event) => setConflictsOnly(event.target.checked)}
          />
          {t("keybindingsConflictsOnly")}
        </label>
        <div className="keybinding-scheme">
          <select
            aria-label={t("keybindingsScheme")}
            value={scheme}
            onChange={(event) => setScheme(event.target.value as KeybindingScheme)}
          >
            {SCHEMES.map((name) => (
              <option key={name}>{name}</option>
            ))}
          </select>
          <button
            type="button"
            disabled={!writable}
            onClick={() => void perform(() => service.import(scheme))}
          >
            {t("keybindingsImportScheme")}
          </button>
        </div>
      </div>
      {(error || state.error) && (
        <div role="alert" className="keybinding-error">
          {error ?? state.error}
          <button type="button" onClick={() => void service.refresh()}>
            {t("keybindingsReload")}
          </button>
        </div>
      )}
      {state.warnings.length > 0 && (
        <div role="status" className="keybinding-warnings">
          {state.warnings.join(" ")}
        </div>
      )}
      {draft && (
        <form
          className="keybinding-draft"
          aria-label={t("keybindingsEditForm", { title: draft.title })}
          onSubmit={(event) => {
            event.preventDefault();
            if (validDraft && writable)
              void perform(() => service.rebind(draft.command, draft.key, draft.when, draft.args));
          }}
        >
          <h2>{draft.title}</h2>
          <label>
            {t("keybindingsShortcut")}
            <div className="keybinding-record-field">
              <input
                ref={shortcutRef}
                aria-label={t("keybindingsShortcut")}
                data-shortcut-recorder="true"
                value={draft.key}
                readOnly={recording}
                onChange={(event) => setDraft({ ...draft, key: event.target.value })}
                onKeyDown={(event) => {
                  if (!recording) return;
                  event.preventDefault();
                  event.stopPropagation();
                  if (event.repeat) return;
                  const chord = eventChord(event.nativeEvent);
                  if (!chord) return;
                  clearTimeout(recordTimer.current);
                  recorded.current = [...recorded.current, chord].slice(0, 4);
                  setDraft({ ...draft, key: recorded.current.join(" ") });
                  recordTimer.current = setTimeout(() => setRecording(false), CHORD_TIMEOUT_MS);
                }}
              />
              <button
                type="button"
                className="keybinding-icon-button"
                aria-label={t("keybindingsRecord")}
                title={t("keybindingsRecord")}
                aria-pressed={recording}
                onClick={record}
              >
                <Icon id="record" label={t("keybindingsRecord")} />
              </button>
            </div>
          </label>
          <label>
            {t("keybindingsWhen")}
            <input
              aria-label={t("keybindingsWhenClause")}
              value={draft.when}
              onChange={(event) => setDraft({ ...draft, when: event.target.value })}
            />
          </label>
          {!validDraft && <p className="keybinding-validation">{t("keybindingsInvalid")}</p>}
          {competing.length > 0 && (
            <section aria-label={t("keybindingsConflicting")} className="keybinding-conflicts">
              <h3>{t("keybindingsCompetingCommands")}</h3>
              {competing.map((binding) => (
                <div key={binding.id}>
                  <span>
                    {titles.get(binding.command) ?? binding.command}{" "}
                    <small>
                      {binding.when
                        ? t("keybindingsOwnerWhen", {
                            owner: binding.owner,
                            when: binding.when,
                          })
                        : binding.owner}
                    </small>
                  </span>
                  <button
                    type="button"
                    disabled={!writable}
                    onClick={() => void perform(() => service.remove(binding), false)}
                  >
                    {t("keybindingsRemoveCompeting")}
                  </button>
                </div>
              ))}
            </section>
          )}
          <div className="keybinding-draft-actions">
            <button type="submit" disabled={!validDraft || !writable || recording}>
              {t("keybindingsSaveBinding")}
            </button>
            <button
              type="button"
              onClick={() => {
                clearTimeout(recordTimer.current);
                setRecording(false);
                setDraft(null);
              }}
            >
              {t("commonCancel")}
            </button>
          </div>
        </form>
      )}
      <div className="keybinding-table-scroll">
        <table aria-label={t("keybindingsTable")}>
          <thead>
            <tr>
              <th>{t("keybindingsCommand")}</th>
              <th>{t("keybindingsShortcut")}</th>
              <th>{t("keybindingsWhen")}</th>
              <th>{t("keybindingsSource")}</th>
              <th>{t("keybindingsActions")}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(({ command, title, binding }) => {
              const competitors = binding ? (conflicts.get(binding.id) ?? []) : [];
              const contenders = binding
                ? state.bindings.filter(
                    (candidate) =>
                      candidate.key === binding.key &&
                      (candidate.id === binding.id || competitors.includes(candidate)),
                  )
                : [];
              const winner = contenders.at(-1);
              return (
                <tr key={binding?.id ?? command}>
                  <td>
                    <strong dir="auto">{title}</strong>
                    <small>{command}</small>
                    {competitors.length > 0 && (
                      <div className="keybinding-row-conflicts">
                        <span>
                          {t("keybindingsCompetes", {
                            commands: competitors
                              .map(
                                (candidate) => titles.get(candidate.command) ?? candidate.command,
                              )
                              .join(", "),
                          })}
                        </span>
                        <span>
                          {t("keybindingsPriority", {
                            command: winner
                              ? (titles.get(winner.command) ?? winner.command)
                              : title,
                          })}
                        </span>
                      </div>
                    )}
                  </td>
                  <td>
                    {binding ? (
                      <kbd>{displayShortcut(binding.key, service.platform)}</kbd>
                    ) : (
                      <span className="keybinding-unassigned">{t("keybindingsUnassigned")}</span>
                    )}
                  </td>
                  <td>
                    <code>{binding?.when ?? t("keybindingsAlways")}</code>
                  </td>
                  <td>
                    {binding?.source === "plugin"
                      ? t("keybindingsPluginSource")
                      : binding?.source === "user"
                        ? t("keybindingsUserSource")
                        : t("keybindingsDefaultSource")}
                    {binding?.source === "plugin" && <small>{binding.owner}</small>}
                  </td>
                  <td>
                    <div className="keybinding-row-actions">
                      <button
                        type="button"
                        className="keybinding-icon-button"
                        aria-label={t("keybindingsChange", { title })}
                        title={t("keybindingsChange", { title })}
                        disabled={!writable}
                        onClick={() => edit(command, title, binding)}
                      >
                        <Icon id="edit" label={t("keybindingsChange", { title })} />
                      </button>
                      <button
                        type="button"
                        className="keybinding-icon-button"
                        aria-label={t("keybindingsReset", { title })}
                        title={t("keybindingsReset", { title })}
                        disabled={!writable}
                        onClick={() => void perform(() => service.resetCommand(command))}
                      >
                        <Icon id="reset" label={t("keybindingsReset", { title })} />
                      </button>
                      {binding && (
                        <button
                          type="button"
                          className="keybinding-icon-button"
                          aria-label={t("keybindingsRemove", { title })}
                          title={t("keybindingsRemove", { title })}
                          disabled={!writable}
                          onClick={() => void perform(() => service.remove(binding))}
                        >
                          <Icon id="close" label={t("keybindingsRemove", { title })} />
                        </button>
                      )}
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        {rows.length === 0 && (
          <p className="keybinding-empty">
            {state.document ? t("keybindingsNoMatch") : t("keybindingsLoading")}
          </p>
        )}
      </div>
      <footer className="keybinding-footer">
        <span>{t("keybindingsCount", { count: rows.length })}</span>
        <span title={state.document?.path ?? undefined}>
          {saving ? t("keybindingsSaving") : (state.document?.path ?? t("keybindingsUserFile"))}
        </span>
      </footer>
    </section>
  );
}
