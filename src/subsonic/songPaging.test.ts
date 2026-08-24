/**
 * Unit tests for the generic page walk over SONGS — the server-side full track index
 * (public issue #5: "Your server has no full track index").
 *
 * There is no "list all songs" endpoint in the Subsonic API. The index comes from `search3`
 * with an empty query, which Navidrome answers with the whole song set, paged by
 * `songOffset`. That makes the termination rule load-bearing in exactly the same way it is
 * for albums, so the album suite's subtle case is re-pinned here for songs rather than
 * assumed to carry over: `walkPages` is shared, but a regression in it would silently
 * truncate a library, and this is the second caller that would suffer it.
 *
 * Pure — no Tauri, no store, in keeping with albumPaging.test.ts.
 */

import { describe, it, expect } from "vitest";
import { walkPages, MAX_SONGS } from "./useSubsonic";
import type { SubSong } from "./nativeSubsonic";

/** `n` synthetic songs, ids offset by `from` so pages are distinguishable. */
function songs(n: number, from = 0): SubSong[] {
  return Array.from({ length: n }, (_, i) => ({
    id: String(from + i),
    title: `Song ${from + i}`,
    artist: "Artist",
    album: "Album",
    duration: 180,
  })) as SubSong[];
}

/**
 * A fake server serving `total` songs.
 * `cap` simulates a server that silently caps page size below what was requested.
 */
function fakeServer(total: number, cap = 500) {
  const calls: number[] = [];
  const fetchPage = async (offset: number) => {
    calls.push(offset);
    return songs(Math.max(0, Math.min(cap, total - offset)), offset);
  };
  return { fetchPage, calls };
}

describe("walkPages over songs", () => {
  it("returns just the first page when the library fits in one page", async () => {
    const { fetchPage, calls } = fakeServer(352); // the reporter's library
    const all = await walkPages(fetchPage, { first: songs(352), max: MAX_SONGS });
    expect(all).toHaveLength(352);
    // One probe past the end is expected — that's how the end is detected.
    expect(calls).toEqual([352]);
  });

  it("pages through a library much larger than one page, without duplicates", async () => {
    const { fetchPage } = fakeServer(2350);
    const all = await walkPages(fetchPage, { first: songs(500), max: MAX_SONGS });
    expect(all).toHaveLength(2350);
    expect(new Set(all.map((s) => s.id)).size).toBe(2350);
  });

  it("does NOT truncate when the server caps page size below what we asked for", async () => {
    // Same regression the album walk exists for: asking 500 and being served 100. Terminating
    // on `page.length < PAGE_SIZE` would stop after ~200 songs of a 1000-song library.
    const { fetchPage } = fakeServer(1000, 100);
    const all = await walkPages(fetchPage, { first: songs(100), max: MAX_SONGS });
    expect(all).toHaveLength(1000);
  });

  it("treats an empty first page as an empty index — the no-empty-query-support case", async () => {
    // A server that reads the empty query literally finds nothing. That is the whole
    // capability probe: an empty walk, no error, and the Tracks section explains itself.
    const { fetchPage, calls } = fakeServer(0);
    const all = await walkPages(fetchPage, { first: [], max: MAX_SONGS });
    expect(all).toEqual([]);
    expect(calls).toEqual([]); // never asks for page 2
  });

  it("stops at the cap so a server that never returns an empty page can't spin forever", async () => {
    const { fetchPage } = fakeServer(Number.MAX_SAFE_INTEGER);
    const all = await walkPages(fetchPage, { first: songs(500), max: 2000 });
    expect(all.length).toBeGreaterThanOrEqual(2000);
    expect(all.length).toBeLessThan(2000 + 500);
  });

  it("abandons the walk when the load generation moves (server switched mid-walk)", async () => {
    const { fetchPage } = fakeServer(5000);
    let stale = false;
    const all = await walkPages(fetchPage, {
      first: songs(500),
      max: MAX_SONGS,
      isStale: () => stale,
      onPage: () => {
        stale = true; // a switchServer landing after the first extra page
      },
    });
    expect(all.length).toBeLessThan(5000);
  });
});
