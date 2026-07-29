/**
 * Unit tests for `walkAlbumPages` — the album pagination walk.
 *
 * EKO previously fetched a single 500-album page and stopped, silently showing only the first
 * 500 albums of libraries that routinely run into the thousands. These tests pin the termination
 * rule, including the subtle case that makes the naive version wrong.
 *
 * Pure — no Tauri, no store, in keeping with serverList.test.ts.
 */

import { describe, it, expect, vi } from "vitest";
import { walkAlbumPages } from "./useSubsonic";
import type { SubAlbum } from "./client";

/** `n` synthetic albums, ids offset by `from` so pages are distinguishable. */
function albums(n: number, from = 0): SubAlbum[] {
  return Array.from({ length: n }, (_, i) => ({
    id: String(from + i),
    name: `Album ${from + i}`,
    artist: "Artist",
  })) as SubAlbum[];
}

/**
 * A fake server serving `total` albums.
 * `cap` simulates a server that silently caps page size below what was requested.
 */
function fakeServer(total: number, cap = 500) {
  const calls: number[] = [];
  const fetchPage = async (offset: number) => {
    calls.push(offset);
    return albums(Math.max(0, Math.min(cap, total - offset)), offset);
  };
  return { fetchPage, calls };
}

describe("walkAlbumPages", () => {
  it("returns just the first page when the library fits in one page", async () => {
    const { fetchPage, calls } = fakeServer(300);
    const all = await walkAlbumPages(fetchPage, { first: albums(300) });
    expect(all).toHaveLength(300);
    // One probe past the end is expected (that's how we detect the end).
    expect(calls).toEqual([300]);
  });

  it("pages through a library much larger than one page", async () => {
    const { fetchPage } = fakeServer(2350);
    const all = await walkAlbumPages(fetchPage, { first: albums(500) });
    expect(all).toHaveLength(2350);
    // No duplicates — offsets advanced correctly.
    expect(new Set(all.map((a) => a.id)).size).toBe(2350);
  });

  it("does NOT truncate when the server caps page size below what we asked for", async () => {
    // THE REGRESSION THIS FIX EXISTS FOR: a server that returns 100 per page even though we
    // asked for 500. Terminating on `page.length < PAGE_SIZE` would stop after ~200 albums.
    const { fetchPage } = fakeServer(1000, 100);
    const all = await walkAlbumPages(fetchPage, { first: albums(100) });
    expect(all).toHaveLength(1000);
  });

  it("handles an empty library without requesting anything further", async () => {
    const { fetchPage, calls } = fakeServer(0);
    const all = await walkAlbumPages(fetchPage, { first: [] });
    expect(all).toEqual([]);
    expect(calls).toEqual([]); // an empty first page means don't even probe
  });

  it("reports progress after each page so the grid can fill in", async () => {
    const { fetchPage } = fakeServer(1500);
    const seen: number[] = [];
    await walkAlbumPages(fetchPage, {
      first: albums(500),
      onPage: (all) => seen.push(all.length),
    });
    expect(seen).toEqual([1000, 1500]);
  });

  it("abandons the walk when isStale flips (server switched mid-load)", async () => {
    const { fetchPage } = fakeServer(5000);
    let pages = 0;
    const all = await walkAlbumPages(fetchPage, {
      first: albums(500),
      isStale: () => ++pages >= 2, // stale on the 2nd fetch
    });
    // Stops early and returns only what it had — does not keep hammering the old server.
    expect(all.length).toBeLessThan(5000);
  });

  it("respects the max cap against a server that never returns an empty page", async () => {
    // Pathological: always returns a full page, so only the cap can stop us.
    const fetchPage = vi.fn(async (offset: number) => albums(500, offset));
    const all = await walkAlbumPages(fetchPage, { first: albums(500), max: 2000 });
    expect(all.length).toBeGreaterThanOrEqual(2000);
    expect(all.length).toBeLessThan(3000);
  });

  it("propagates a fetch error so the caller can keep the pages already loaded", async () => {
    const fetchPage = async () => {
      throw new Error("network");
    };
    await expect(walkAlbumPages(fetchPage, { first: albums(500) })).rejects.toThrow("network");
  });
});
