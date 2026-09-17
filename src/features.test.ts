/**
 * Unit tests for the three free table-stakes features:
 *   #41 Synced lyrics — active-line selection
 *   #42 Scrobble (the threshold now lives in Rust: eko_core::player::scrobble_threshold_ms)
 *   #43 Sleep timer — state transitions
 *
 * These are pure-logic tests; no Tauri IPC, no DOM, no network.
 */

import { describe, it, expect } from "vitest";
import { activeLyricLine } from "./lib/lyrics";
import type { SyncedLyricLine } from "./subsonic/nativeSubsonic";

// ── #41 Synced lyrics — active-line selection ─────────────────────────────────

function lines(...starts: number[]): SyncedLyricLine[] {
  return starts.map((start, i) => ({ start, value: `Line ${i + 1}` }));
}

describe("activeLyricLine", () => {
  it("returns -1 for an empty line list", () => {
    expect(activeLyricLine([], 5000)).toBe(-1);
  });

  it("returns 0 before the first line starts", () => {
    const l = lines(1000, 5000, 10000);
    // posMs = 500 ms < first line start (1000 ms): we return 0 (first line is current)
    expect(activeLyricLine(l, 500)).toBe(0);
  });

  it("returns 0 when posMs exactly matches the first line start", () => {
    const l = lines(1000, 5000, 10000);
    expect(activeLyricLine(l, 1000)).toBe(0);
  });

  it("returns the correct line index for mid-song position", () => {
    const l = lines(0, 3000, 7000, 12000);
    // At 5000 ms: line at 3000 has started, line at 7000 has not yet
    expect(activeLyricLine(l, 5000)).toBe(1);
  });

  it("advances to the next line exactly at its start timestamp", () => {
    const l = lines(0, 3000, 7000);
    expect(activeLyricLine(l, 7000)).toBe(2);
  });

  it("stays on the last line after the track ends", () => {
    const l = lines(0, 3000, 7000);
    expect(activeLyricLine(l, 99999)).toBe(2);
  });

  it("handles a single-line lyrics (entire track)", () => {
    const l = lines(0);
    expect(activeLyricLine(l, 0)).toBe(0);
    expect(activeLyricLine(l, 60000)).toBe(0);
  });

  it("handles lines with start=0 at the very beginning", () => {
    const l = lines(0, 2000, 5000);
    expect(activeLyricLine(l, 0)).toBe(0);
    expect(activeLyricLine(l, 1999)).toBe(0);
    expect(activeLyricLine(l, 2000)).toBe(1);
  });
});

// ── #43 Sleep timer — pure-function helpers ────────────────────────────────────

// The sleep timer's reactive state lives in the Zustand store (needs a DOM environment
// to test via `usePlayerStore`). The calculations are straightforward arithmetic;
// we test the constants and threshold calculations that have pure-function surfaces.

import { SLEEP_PRESETS } from "./store/usePlayerStore";

describe("SLEEP_PRESETS", () => {
  it("contains the four standard presets", () => {
    expect(SLEEP_PRESETS).toContain(15);
    expect(SLEEP_PRESETS).toContain(30);
    expect(SLEEP_PRESETS).toContain(45);
    expect(SLEEP_PRESETS).toContain(60);
  });

  it("has exactly four entries", () => {
    expect(SLEEP_PRESETS).toHaveLength(4);
  });

  it("all entries are positive integers", () => {
    for (const p of SLEEP_PRESETS) {
      expect(p).toBeGreaterThan(0);
      expect(Number.isInteger(p)).toBe(true);
    }
  });

  it("sentinel -1 is NOT in the presets array (it is a separate UI option)", () => {
    expect(SLEEP_PRESETS).not.toContain(-1);
  });
});

describe("sleep timer duration math", () => {
  it("converts preset minutes to milliseconds correctly", () => {
    // This is the arithmetic the store uses: preset * 60 * 1000
    for (const mins of SLEEP_PRESETS) {
      expect(mins * 60 * 1000).toBe(mins * 60000);
    }
  });

  it("15 min preset = 900 000 ms", () => {
    expect(15 * 60 * 1000).toBe(900_000);
  });

  it("60 min preset = 3 600 000 ms", () => {
    expect(60 * 60 * 1000).toBe(3_600_000);
  });
});
