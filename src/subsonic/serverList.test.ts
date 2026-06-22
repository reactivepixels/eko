/**
 * Unit tests for the pure (no-Tauri) server list logic in serverList.ts.
 *
 * These test applyAddServer / applyRemoveServer / applyRenameServer /
 * resolveActiveAfterRemove / buildMigratedList without any mock of `invoke`.
 */

import { describe, it, expect } from "vitest";
import {
  applyAddServer,
  applyRemoveServer,
  applyRenameServer,
  resolveActiveAfterRemove,
  buildMigratedList,
  type ServerEntry,
} from "./serverList";

// ── Fixtures ──────────────────────────────────────────────────────────────────

function makeEntry(id: string, name: string, baseUrl = "http://host:4533"): ServerEntry {
  return { id, name, baseUrl, username: "admin" };
}

// ── applyAddServer ────────────────────────────────────────────────────────────

describe("applyAddServer", () => {
  it("appends a new entry to an empty list", () => {
    const entry = makeEntry("a", "Home");
    const result = applyAddServer([], entry);
    expect(result).toHaveLength(1);
    expect(result[0]).toEqual(entry);
  });

  it("appends to an existing list without mutating the original", () => {
    const original = [makeEntry("a", "Home")];
    const entry = makeEntry("b", "Office");
    const result = applyAddServer(original, entry);
    expect(result).toHaveLength(2);
    expect(result[1]).toEqual(entry);
    // Original unchanged
    expect(original).toHaveLength(1);
  });

  it("allows multiple servers with the same URL but different ids", () => {
    const a = makeEntry("a", "Home", "http://same:4533");
    const b = makeEntry("b", "Also Home", "http://same:4533");
    const result = applyAddServer([a], b);
    expect(result).toHaveLength(2);
  });
});

// ── applyRemoveServer ─────────────────────────────────────────────────────────

describe("applyRemoveServer", () => {
  it("removes the matching entry by id", () => {
    const servers = [makeEntry("a", "Home"), makeEntry("b", "Office")];
    const result = applyRemoveServer(servers, "a");
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("b");
  });

  it("returns empty list when removing the only entry", () => {
    const servers = [makeEntry("a", "Home")];
    const result = applyRemoveServer(servers, "a");
    expect(result).toHaveLength(0);
  });

  it("returns unchanged list when id does not exist", () => {
    const servers = [makeEntry("a", "Home")];
    const result = applyRemoveServer(servers, "nonexistent");
    expect(result).toHaveLength(1);
  });

  it("does not mutate the original array", () => {
    const original = [makeEntry("a", "Home"), makeEntry("b", "Office")];
    applyRemoveServer(original, "a");
    expect(original).toHaveLength(2);
  });
});

// ── applyRenameServer ─────────────────────────────────────────────────────────

describe("applyRenameServer", () => {
  it("renames the matching entry", () => {
    const servers = [makeEntry("a", "Home"), makeEntry("b", "Office")];
    const result = applyRenameServer(servers, "a", "Castle");
    expect(result[0].name).toBe("Castle");
    expect(result[1].name).toBe("Office");
  });

  it("returns unchanged list when id does not exist", () => {
    const servers = [makeEntry("a", "Home")];
    const result = applyRenameServer(servers, "nope", "New Name");
    expect(result[0].name).toBe("Home");
  });

  it("does not mutate the original array", () => {
    const original = [makeEntry("a", "Home")];
    applyRenameServer(original, "a", "New");
    expect(original[0].name).toBe("Home");
  });
});

// ── resolveActiveAfterRemove ──────────────────────────────────────────────────

describe("resolveActiveAfterRemove", () => {
  it("keeps the current active id when a different server is removed", () => {
    const remaining = [makeEntry("a", "Home"), makeEntry("b", "Office")];
    const result = resolveActiveAfterRemove(remaining, "b", "a");
    expect(result).toBe("a");
  });

  it("moves to the first remaining server when the active server is removed", () => {
    const remaining = [makeEntry("b", "Office"), makeEntry("c", "Cloud")];
    const result = resolveActiveAfterRemove(remaining, "a", "a");
    expect(result).toBe("b");
  });

  it("returns null when the active server is removed and the list is empty", () => {
    const result = resolveActiveAfterRemove([], "a", "a");
    expect(result).toBeNull();
  });

  it("handles null activeId gracefully", () => {
    const remaining = [makeEntry("a", "Home")];
    const result = resolveActiveAfterRemove(remaining, "a", null);
    expect(result).toBeNull();
  });
});

// ── buildMigratedList ─────────────────────────────────────────────────────────

describe("buildMigratedList", () => {
  it("creates a single-entry list with a hostname-derived label", () => {
    const result = buildMigratedList({
      baseUrl: "http://192.168.1.10:4533",
      username: "admin",
    });
    expect(result).toHaveLength(1);
    expect(result[0].id).toBe("migrated");
    expect(result[0].name).toBe("192.168.1.10:4533");
    expect(result[0].baseUrl).toBe("http://192.168.1.10:4533");
    expect(result[0].username).toBe("admin");
  });

  it("uses the full URL as the label when it is not parseable as a URL", () => {
    const result = buildMigratedList({
      baseUrl: "not-a-url",
      username: "rod",
    });
    expect(result[0].name).toBe("not-a-url");
  });

  it("handles a URL with a path prefix gracefully", () => {
    const result = buildMigratedList({
      baseUrl: "https://navidrome.example.com",
      username: "alice",
    });
    expect(result[0].name).toBe("navidrome.example.com");
  });
});

// ── Edge-case integration: add then remove ────────────────────────────────────

describe("add → remove flow (pure)", () => {
  it("round-trips correctly", () => {
    const a = makeEntry("a", "Home");
    const b = makeEntry("b", "Office");
    let servers: ServerEntry[] = [];
    servers = applyAddServer(servers, a);
    servers = applyAddServer(servers, b);
    expect(servers).toHaveLength(2);
    servers = applyRemoveServer(servers, "a");
    expect(servers).toHaveLength(1);
    expect(servers[0].id).toBe("b");
  });

  it("rename after add is reflected", () => {
    const a = makeEntry("a", "Home");
    let servers = applyAddServer([], a);
    servers = applyRenameServer(servers, "a", "Castle");
    expect(servers[0].name).toBe("Castle");
  });
});
