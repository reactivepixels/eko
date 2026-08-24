/**
 * Unit tests for `useSubsonic.loadSongs` — the lazy server track index (public issue #5).
 *
 * The walk is one request for a small library and ~200 for a very large one, so *when* it
 * runs is a real cost decision, and "it already ran" has to be remembered accurately. Two
 * failure modes are pinned here because both are invisible in normal use and expensive:
 *
 *  * re-walking on every visit to the Tracks section, and
 *  * re-walking forever against a server whose index is legitimately empty (the servers
 *    that don't answer an empty `search3` query) — where `songs.length` as the guard would
 *    never latch, so `songsLoaded` is the guard instead.
 *
 * `getSongs` is mocked at the IPC wrapper, so no Tauri and no socket.
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import type { SubSong } from "./nativeSubsonic";
// `?raw` (declared by `vite/client`) — the structural guard at the bottom reads the source.
import storeSource from "./useSubsonic.ts?raw";

const { getSongs } = vi.hoisted(() => ({ getSongs: vi.fn() }));
vi.mock("./nativeSubsonic", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./nativeSubsonic")>()),
  getSongs,
}));

const { useSubsonic } = await import("./useSubsonic");

/** `n` synthetic wire songs, ids offset by `from`. */
function songs(n: number, from = 0): SubSong[] {
  return Array.from({ length: n }, (_, i) => ({
    id: String(from + i),
    title: `Song ${from + i}`,
    artist: "Artist",
    album: "Album",
    duration: 180,
  })) as SubSong[];
}

/** A server holding `total` songs, served in pages of 500. */
function serve(total: number) {
  getSongs.mockImplementation(async (size: number, offset: number) =>
    songs(Math.max(0, Math.min(size, total - offset)), offset),
  );
}

beforeEach(() => {
  getSongs.mockReset();
  useSubsonic.setState({ songs: [], songsLoading: false, songsLoaded: false });
});

describe("loadSongs", () => {
  it("walks the whole index and converts the wire songs to tracks", async () => {
    serve(1200);
    await useSubsonic.getState().loadSongs();
    const s = useSubsonic.getState();
    expect(s.songs).toHaveLength(1200);
    expect(s.songsLoaded).toBe(true);
    expect(s.songsLoading).toBe(false);
    // Converted, not passed through raw: a server track carries its id in `subsonicId`.
    expect(s.songs[0].subsonicId).toBe("0");
    expect(new Set(s.songs.map((t) => t.id)).size).toBe(1200);
  });

  it("does not re-walk on a second call — the section can be re-entered for free", async () => {
    serve(600);
    await useSubsonic.getState().loadSongs();
    const callsAfterFirst = getSongs.mock.calls.length;
    expect(callsAfterFirst).toBeGreaterThan(0);

    await useSubsonic.getState().loadSongs();
    expect(getSongs.mock.calls.length).toBe(callsAfterFirst);
  });

  it("treats an empty index as final, so a server that can't do it isn't re-asked forever", async () => {
    serve(0);
    await useSubsonic.getState().loadSongs();
    expect(useSubsonic.getState().songs).toEqual([]);
    expect(useSubsonic.getState().songsLoaded).toBe(true);
    const calls = getSongs.mock.calls.length;

    await useSubsonic.getState().loadSongs();
    expect(getSongs.mock.calls.length).toBe(calls);
  });

  it("keeps the pages that landed when the walk fails part-way", async () => {
    getSongs.mockResolvedValueOnce(songs(500)).mockRejectedValueOnce(new Error("connection reset"));
    await useSubsonic.getState().loadSongs();
    const s = useSubsonic.getState();
    // A partial index beats an error screen — and the spinner must still resolve.
    expect(s.songs).toHaveLength(500);
    expect(s.songsLoading).toBe(false);
    expect(s.songsLoaded).toBe(true);
  });
});

/**
 * The index is per-server state, and the two connect paths (`connect` and `addAndConnect`)
 * each write their own `set({ connected: true, ... })` block. When the reset list was inline
 * it was duplicated, and adding the index to only one of them let a second server inherit the
 * first server's tracks — invisible, because `songsLoaded` was already true so no walk ran to
 * correct it. `freshLibrary()` is now the single definition; this pins that every connect path
 * uses it, which is the part a future edit could quietly drop.
 */
describe("per-server reset", () => {
  // Every `set({ ... })` call in the store, by its body. Comments and doc blocks sit outside
  // these, so the phrase `connected: true` in prose can't masquerade as a state write.
  const setBodies = [...storeSource.matchAll(/set\(\{([\s\S]*?)\n\s*\}\);/g)].map((m) => m[1]);

  it("spreads freshLibrary() in every block that flips connected: true", () => {
    const connects = setBodies.filter((b) => b.includes("connected: true"));
    expect(connects.length).toBeGreaterThanOrEqual(2); // connect + addAndConnect
    for (const body of connects) expect(body).toContain("...freshLibrary()");
  });

  it("resets the index on disconnect too, so a switch can't show the old server's tracks", () => {
    // `switchServer` disconnects before reconnecting, so this is the reset that stops the
    // outgoing server's tracks being visible while the incoming one loads. Scoped to the
    // action body (ends at the next top-level `},`) rather than to any `connected: false`
    // block — the connect ERROR paths also set that, and they have nothing to reset.
    const from = storeSource.slice(storeSource.indexOf("disconnect: async"));
    expect(from.slice(0, from.indexOf("\n  },"))).toContain("...freshLibrary()");
  });
});
