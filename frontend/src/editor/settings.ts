export interface EditorSettings {
  minimap: boolean;
  bracketColorization: boolean;
  indentGuides: boolean;
  lineNumbers: "on" | "off" | "relative";
  wordWrap: "off" | "on" | "wordWrapColumn" | "bounded";
  renderWhitespace: "none" | "boundary" | "selection" | "all";
  folding: boolean;
}

export const DEFAULT_EDITOR_SETTINGS: EditorSettings = {
  minimap: true,
  bracketColorization: true,
  indentGuides: true,
  lineNumbers: "on",
  wordWrap: "off",
  renderWhitespace: "selection",
  folding: true,
};