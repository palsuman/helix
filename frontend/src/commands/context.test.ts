import { describe, expect, it } from "vitest";
import { evaluateEnablement } from "./context";

describe("command enablement", () => {
  const context = {
    editorTextFocus: true,
    terminalFocus: false,
    languageId: "typescript",
    editorCount: 2,
  };

  it("evaluates boolean, comparison, negation, and grouping expressions", () => {
    expect(evaluateEnablement("editorTextFocus && !terminalFocus", context)).toBe(true);
    expect(evaluateEnablement("languageId == 'typescript' && editorCount != 1", context)).toBe(
      true,
    );
    expect(evaluateEnablement("terminalFocus || (editorTextFocus && false)", context)).toBe(false);
    expect(evaluateEnablement("editorTextFocus || missingContext", context)).toBe(true);
    expect(evaluateEnablement("terminalFocus && missingContext", context)).toBe(false);
  });

  it("fails closed for invalid expressions and missing keys", () => {
    expect(evaluateEnablement("missingContext", context)).toBe(false);
    expect(evaluateEnablement("editorTextFocus &&", context)).toBe(false);
  });

  it("supports unquoted comparison values without treating missing keys as equal", () => {
    expect(evaluateEnablement("editorLangId == typescript", { editorLangId: "typescript" })).toBe(
      true,
    );
    expect(evaluateEnablement("editorLangId == typescript", {})).toBe(false);
    expect(evaluateEnablement("editorLangId != typescript", { editorLangId: "rust" })).toBe(true);
  });
});
