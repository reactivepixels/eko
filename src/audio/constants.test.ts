/**
 * Pins the graphic-EQ preset table.
 *
 * The same table exists in Rust — `src-tauri/crates/eko-core/src/eq_presets.rs`, for
 * the terminal client — and this TypeScript copy remains because it is a synchronous
 * skin data-feed binding (`constants.EQ_PRESETS`). A changed gain is an AUDIBLE change,
 * so both sides are pinned: Rust's `preset_values_are_the_ported_table` and this file.
 * Changing a preset therefore requires editing two tables and two tests — deliberately.
 *
 * Pure-logic test — no Tauri IPC, no DOM.
 */

import { describe, it, expect } from "vitest";
import {
  EQ_BANDS,
  EQ_BAND_COUNT,
  EQ_BAND_LABELS,
  EQ_GAIN_MIN,
  EQ_GAIN_MAX,
  EQ_Q,
  EQ_PRESETS,
  FLAT_GAINS,
} from "./constants";

/** Transcribed from `eko_core::eq_presets::PRESETS`, in menu order. */
const RUST_PRESETS: [string, number, number[]][] = [
  ["Flat", 0, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
  ["Rock", -5, [5, 3, -1, -2, -1, 1, 3, 4, 5, 5]],
  ["Pop", -3, [-1, 1, 3, 4, 3, 1, -1, -1, 0, 1]],
  ["Bass Boost", -5, [6, 5, 4, 2, 0, 0, 0, 0, 0, 0]],
  ["Treble Boost", -4, [0, 0, 0, 0, 0, 1, 3, 4, 5, 5]],
  ["Vocal", -3, [-2, -2, 0, 2, 4, 4, 3, 1, 0, -1]],
  ["Jazz", -3, [3, 2, 0, 1, -1, -1, 0, 1, 2, 3]],
  ["Acoustic", -3, [3, 3, 1, 0, 1, 1, 2, 2, 1, 1]],
  ["Classical", -2, [3, 2, 0, 0, 0, 0, -1, -1, -2, -3]],
  ["Loudness", -5, [6, 4, 0, -1, -2, -1, 0, 3, 5, 5]],
];

describe("EQ presets", () => {
  it("the preset table matches the Rust source of truth", () => {
    expect(EQ_PRESETS.map((p) => [p.name, p.preamp, p.gains])).toEqual(RUST_PRESETS);
  });

  it("every preset carries exactly one gain per band", () => {
    expect(EQ_BAND_COUNT).toBe(10);
    for (const p of EQ_PRESETS) {
      expect(p.gains, `${p.name} must have ${EQ_BAND_COUNT} gains`).toHaveLength(EQ_BAND_COUNT);
    }
  });

  it("every gain sits inside the slider range", () => {
    for (const p of EQ_PRESETS) {
      for (const g of p.gains) {
        expect(g, `${p.name} gain out of range`).toBeGreaterThanOrEqual(EQ_GAIN_MIN);
        expect(g, `${p.name} gain out of range`).toBeLessThanOrEqual(EQ_GAIN_MAX);
      }
    }
  });

  it("Flat is the only preset that touches nothing", () => {
    for (const p of EQ_PRESETS) {
      const inert = p.preamp === 0 && p.gains.every((g) => g === 0);
      expect(inert, `${p.name} inertness`).toBe(p.name === "Flat");
    }
  });

  it("any preset that boosts buys headroom (overlapping broad bands would clip)", () => {
    for (const p of EQ_PRESETS) {
      if (p.gains.some((g) => g > 0)) {
        expect(p.preamp, `${p.name} boosts without headroom`).toBeLessThan(0);
      }
    }
  });

  it("band frequencies, labels and Q match the Rust table", () => {
    expect([...EQ_BANDS]).toEqual([60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000]);
    expect(EQ_BAND_LABELS).toEqual([
      "60",
      "170",
      "310",
      "600",
      "1k",
      "3k",
      "6k",
      "12k",
      "14k",
      "16k",
    ]);
    expect(EQ_Q).toBe(1.0);
    expect(FLAT_GAINS()).toEqual([0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  });
});
