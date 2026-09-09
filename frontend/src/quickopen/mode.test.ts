import { describe, expect, it } from "vitest";
import { parseLineNumber, parseQuickOpenQuery } from "./mode";

describe("quick open modes", () => {
  it.each([
    ["file.ts", "files", "file.ts"],
    ["@render", "documentSymbols", "render"],
    ["#Workspace", "workspaceSymbols", "Workspace"],
    [":42", "line", "42"],
    [">format", "commands", "format"],
  ] as const)("parses %s", (value, mode, query) => {
    expect(parseQuickOpenQuery(value)).toEqual({ mode, query });
  });

  it("accepts only positive integer line numbers", () => {
    expect(parseLineNumber("42")).toBe(42);
    expect(parseLineNumber("0")).toBeNull();
    expect(parseLineNumber("4.2")).toBeNull();
  });
});
