import { describe, expect, it } from "vitest";
import { findMatches, replaceAllMatches, replaceMatch } from "./findReplace";

describe("single-file find and replace", () => {
  it("supports case-sensitive, whole-word, and regex searches", () => {
    expect(findMatches("Cat cat cater", { query: "cat", regex: false, caseSensitive: false, wholeWord: true })).toHaveLength(2);
    expect(findMatches("Cat cat", { query: "cat", regex: false, caseSensitive: true, wholeWord: false })).toHaveLength(1);
    expect(findMatches("item1 item22", { query: "item\\d+", regex: true, caseSensitive: true, wholeWord: false })).toHaveLength(2);
  });

  it("limits matches to a selection and replaces one or all matches", () => {
    const text = "one two one";
    const matches = findMatches(text, { query: "one", regex: false, caseSensitive: false, wholeWord: true }, { start: 8, end: 11 });
    expect(matches).toEqual([{ start: 8, end: 11 }]);
    expect(replaceMatch(text, matches[0]!, "TWO")).toBe("one two TWO");
    expect(replaceAllMatches(text, findMatches(text, { query: "one", regex: false, caseSensitive: false, wholeWord: true }), "ONE")).toBe("ONE two ONE");
  });

  it("returns no matches for invalid regular expressions", () => {
    expect(findMatches("text", { query: "[", regex: true, caseSensitive: false, wholeWord: false })).toEqual([]);
  });
});