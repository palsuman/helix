import { describe, expect, it } from "vitest";
import { createMockIpc } from "../test/mockIpc";
import { search, searchStats } from "./commands";

describe("search client", () => {
  it("dispatches typed search queries", async () => {
    const kernel = createMockIpc();
    kernel.respond("search.query", { matches: [] });
    await expect(search(kernel.client, {
      root: "/workspace",
      query: "needle",
      case_sensitive: true,
      whole_word: false,
      max_results: 20,
      regex: false,
      include_glob: null,
      exclude_glob: null,
      context_lines: 0,
      respect_gitignore: true,
    })).resolves.toEqual({ matches: [] });
    expect(kernel.requests[0]?.command).toBe("search.query");
  });

  it("requests index statistics", async () => {
    const kernel = createMockIpc();
    kernel.respond("search.stats", { stats: { indexed_files: 2, indexed_bytes: 10, truncated: false } });
    await expect(searchStats(kernel.client, "/workspace")).resolves.toMatchObject({ stats: { indexed_files: 2 } });
  });
});