/**
 * The shared `SubSong` → `Track` behavioural contract, defined **once** and asserted against
 * every implementation of it.
 *
 * There are two implementations, deliberately not merged: `toTrack` (`useSubsonic.ts`, free)
 * serves the library, search and playlists; `subSongToTrack` (`pro/useSmartPlaylistStore.ts`,
 * Pro) serves smart playlists and Instant Mix. Merging them would close a module cycle —
 * `useSubsonic.ts` pulls in `usePlayerStore`, which reaches back into `@pro`.
 *
 * **Why the assertions live here rather than in one test file.** They used to, and that file
 * imported the Pro twin by relative path (`../pro/useSmartPlaylistStore`). The publish pipeline
 * excludes `src/pro/` but keeps every other test file, so the published free repo shipped a test
 * importing a module that is not there — breaking `npm run typecheck` and `vitest` in the public
 * MIT repo. (`cargo build` + a typecheck against the published subset would not catch it; only
 * running the suite does.)
 *
 * Two obvious fixes both fail:
 *   - **Import the twin through `@pro`.** Vitest resolves `@pro` to `src/pro-stub/`, so the Pro
 *     implementation would be unreachable and the twin assertions would silently vanish from
 *     the dev tree's own test run — losing the coverage this contract exists to provide.
 *   - **Duplicate the assertions into a Pro test file.** The entire point of this suite is that
 *     the twins cannot drift; two hand-maintained copies of the assertions reintroduce exactly
 *     that risk one level up.
 *
 * So: assertions here, once. `src/subsonic/toTrack.test.ts` runs them against the free twin and
 * ships publicly; `src/pro/subSongToTrack.test.ts` runs them against the Pro twin *and* pins
 * the two field-for-field, and disappears with the rest of `src/pro/` on publish. Both trees
 * end up guarded and both resolve.
 *
 * Not a `.test.ts` file, so vitest's `src/**\/*.test.ts` glob does not collect it directly. No
 * app code imports it, so it never reaches a bundle.
 */

import { describe, it, expect } from "vitest";
import type { SubSong } from "./nativeSubsonic";
import type { DownloadUrl, StreamSrcUrl, Track } from "../types";

/** A `SubSong` → `Track` normaliser. Both twins have this shape. */
export type Normaliser = (s: SubSong) => Track;

/** A minimally-populated song, as `eko-net` would serialise a fully-untagged track. */
export function untagged(overrides: Partial<SubSong> = {}): SubSong {
  return { id: "song-1", title: "", artist: "", album: "", ...overrides };
}

/**
 * A branded pair for the URL pass-through case.
 *
 * These two casts, and the ones in `agreementCases`, are the only ones in the tree, and they
 * belong here: `StreamSrcUrl` / `DownloadUrl` are phantom-branded so that no *production* code
 * can mint or swap one, and the sole legitimate producer is `invoke<SubSong>()` — a
 * declaration-only boundary TypeScript takes on faith. These fixtures stand in for that
 * boundary, so they have to assert the brand exactly as the wire would. A cast anywhere in
 * `src/` outside a fixture is a bug, not a convenience.
 */
export const STREAM_SRC_URL = "https://music.example.com/rest/stream?id=song-1" as StreamSrcUrl;
export const DOWNLOAD_URL = "https://music.example.com/rest/download?id=song-1" as DownloadUrl;

/**
 * Every behavioural assertion for one normaliser.
 *
 * The reason this contract exists at all is one operator. `eko-net` types `SubSong.title` /
 * `.artist` / `.album` as `String` with `#[serde(default)]`, so an untagged track crosses the
 * IPC boundary as `""` where the old TypeScript client left the field `undefined`. `??` is
 * *nullish* coalescing — `"" ?? null` is `""` — so a `??` at this boundary would let the empty
 * string through, and every downstream placeholder is itself a `??`:
 *
 *   useNowPlaying.ts       `track?.title ?? "EKO"`
 *   usePlayerStore.ts      `t?.title ?? "EKO"`      (mini window + native now-playing)
 *   usePlayerStore.ts      `t.title ?? "Unknown"`   (macOS Control Center / lock screen)
 *   DeckShell.tsx          `?? "—"`
 *   QueuePanel.tsx         `"X" by "Y"` aria-label, `?? "—"`, `?? "track"`
 *
 * All seven would silently render blank. Nothing in `src/` compares these fields against
 * `null`, so normalising `""` → `null` at this single boundary restores byte-exact parity
 * everywhere at once — and these assertions are what stop a future "tidy-up" turning the `||`
 * back into a `??`.
 *
 * Pure — no Tauri, no store.
 */
export function describeNormaliserContract(name: string, norm: Normaliser): void {
  describe(`${name} — empty display fields normalise to null`, () => {
    it("maps an untagged song's title/artist/album to null, not the empty string", () => {
      const t = norm(untagged());
      expect(t.title).toBeNull();
      expect(t.artist).toBeNull();
      expect(t.album).toBeNull();
    });

    it("leaves the downstream `?? placeholder` fallbacks reachable", () => {
      // This is the actual regression: with `""` these expressions evaluate to `""`.
      const t = norm(untagged());
      expect(t.title ?? "EKO").toBe("EKO");
      expect(t.artist ?? "Unknown").toBe("Unknown");
      expect(t.album ?? "—").toBe("—");
    });

    it("normalises each field independently", () => {
      const t = norm(untagged({ title: "Reflektor", album: "Reflektor" }));
      expect(t.title).toBe("Reflektor");
      expect(t.artist).toBeNull(); // still untagged
      expect(t.album).toBe("Reflektor");
    });

    it("passes tagged values through untouched", () => {
      const t = norm(untagged({ title: "Song", artist: "Artist", album: "Album" }));
      expect(t.title).toBe("Song");
      expect(t.artist).toBe("Artist");
      expect(t.album).toBe("Album");
    });

    it('does not treat a whitespace-only tag as empty (only `""` normalises)', () => {
      // `||` is falsiness, not trimming — a server that tags a track " " keeps it, exactly
      // as the old client did. Pinned so nobody "improves" this into a `.trim()`.
      const t = norm(untagged({ title: " " }));
      expect(t.title).toBe(" ");
    });
  });

  describe(`${name} — the pre-signed URLs are carried, not rebuilt`, () => {
    it("copies streamSrcUrl / downloadUrl / coverUrl off the SubSong", () => {
      const t = norm(
        untagged({
          streamSrcUrl: STREAM_SRC_URL,
          downloadUrl: DOWNLOAD_URL,
          coverUrl: "stream://localhost/?src=enc",
        }),
      );
      expect(t.streamSrcUrl).toBe("https://music.example.com/rest/stream?id=song-1");
      expect(t.downloadUrl).toBe("https://music.example.com/rest/download?id=song-1");
      expect(t.coverUrl).toBe("stream://localhost/?src=enc");
    });

    it("leaves them undefined when the payload carries none", () => {
      const t = norm(untagged());
      expect(t.streamSrcUrl).toBeUndefined();
      expect(t.downloadUrl).toBeUndefined();
      expect(t.coverUrl).toBeUndefined();
    });
  });

  describe(`${name} — numeric fields keep \`??\`, where 0 is a real value`, () => {
    it("preserves a zero duration rather than coalescing it away", () => {
      // `??` is correct for these: `0 || 0` and `0 ?? 0` agree here, but a future
      // `duration || 3` would not. Pinned to keep the two operators' roles distinct.
      const t = norm(untagged({ duration: 0 }));
      expect(t.duration).toBe(0);
    });

    it("defaults an absent duration to 0 and absent channels to 2", () => {
      const t = norm(untagged());
      expect(t.duration).toBe(0);
      expect(t.channels).toBe(2);
    });
  });
}

/**
 * Songs for the field-for-field agreement check between two twins. Only meaningful where both
 * exist, so the comparison itself lives in the Pro test file.
 */
export const agreementCases: Array<[string, SubSong]> = [
  ["fully untagged", untagged()],
  [
    "fully populated",
    untagged({
      title: "Angel",
      artist: "Massive Attack",
      album: "Mezzanine",
      duration: 379,
      bitRate: 1005,
      samplingRate: 44100,
      channelCount: 2,
      track: 1,
      suffix: "flac",
      contentType: "audio/flac",
      coverArt: "mf-1",
      replayGain: { trackGain: -7.2, albumGain: -6.85, trackPeak: 0.977, albumPeak: 1 },
      streamSrcUrl: STREAM_SRC_URL,
      downloadUrl: DOWNLOAD_URL,
      coverUrl: "stream://localhost/?src=enc",
    }),
  ],
  ["zero duration, whitespace title", untagged({ duration: 0, title: " " })],
  ["no replayGain, no suffix", untagged({ title: "T", replayGain: undefined })],
];
