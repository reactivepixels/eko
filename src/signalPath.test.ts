/**
 * Unit tests for the bit-perfect seal contract and skin gating.
 *
 * `isBitPerfect` is the single source of truth for whether playback is untouched —
 * any EQ, volume attenuation, ReplayGain, or resampling (engine or OS device) breaks it.
 *
 * Pure-logic tests — no Tauri IPC, no DOM.
 */

import { describe, it, expect } from "vitest";
import { isBitPerfect, type SignalFlags } from "./hooks/useSignalPath";
import { isProSkin, SKINS } from "./store/useUiStore";

const clean: SignalFlags = {
  eqActive: false,
  attenuated: false,
  rgActive: false,
  resampled: false,
  osResampled: false,
};

describe("isBitPerfect", () => {
  it("is true only when no signal-path modifier is engaged", () => {
    expect(isBitPerfect(clean)).toBe(true);
  });

  const modifiers: (keyof SignalFlags)[] = [
    "eqActive",
    "attenuated",
    "rgActive",
    "resampled",
    "osResampled",
  ];

  for (const key of modifiers) {
    it(`${key} engaged → not bit-perfect`, () => {
      expect(isBitPerfect({ ...clean, [key]: true })).toBe(false);
    });
  }

  it("any combination of modifiers is not bit-perfect", () => {
    expect(isBitPerfect({ ...clean, eqActive: true, attenuated: true })).toBe(false);
    expect(isBitPerfect({ ...clean, rgActive: true, resampled: true })).toBe(false);
  });
});

describe("isProSkin", () => {
  it("porcelain is the only free skin", () => {
    expect(isProSkin("porcelain")).toBe(false);
  });

  it("alternate skins are Pro-gated", () => {
    expect(isProSkin("studio")).toBe(true);
    expect(isProSkin("aether")).toBe(true);
  });

  it("exactly one registered skin is free", () => {
    const free = SKINS.filter((s) => !isProSkin(s.id));
    expect(free.map((s) => s.id)).toEqual(["porcelain"]);
  });
});
