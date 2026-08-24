/**
 * The GUI half of the bit-perfect seal contract.
 *
 * The derivation itself is Rust's (`crates/eko-core/src/signal_path.rs` holds the
 * cases that used to live in `src/signalPath.test.ts`). What is still the frontend's
 * responsibility — and what this file guards — is the state BEFORE a derivation has
 * landed. `useSignalPath` is a synchronous read of an asynchronously-derived seal, so
 * there is necessarily a moment with nothing to show. That moment must render nothing.
 *
 * A seal that showed BIT-PERFECT while the signal was actually being resampled would
 * be a lying seal, and the seal is the product's central claim.
 *
 * Pure-logic test — no Tauri IPC, no DOM.
 */

import { describe, it, expect } from "vitest";
import { NO_SEAL } from "./useSignalPath";

describe("the seal before a derivation lands", () => {
  it("claims nothing — not bit-perfect, and not displayable", () => {
    expect(NO_SEAL.pure).toBe(false);
    expect(NO_SEAL.active).toBe(false);
  });

  it("names no modifier, so no breakdown can be shown either", () => {
    expect(NO_SEAL.eqActive).toBe(false);
    expect(NO_SEAL.attenuated).toBe(false);
    expect(NO_SEAL.rgActive).toBe(false);
    expect(NO_SEAL.resampled).toBe(false);
    expect(NO_SEAL.osResampled).toBe(false);
  });

  it("carries no display string that could be mistaken for a verdict", () => {
    for (const [key, value] of Object.entries(NO_SEAL)) {
      if (typeof value === "string") {
        expect(value, `${key} must be empty until Rust derives it`).toBe("");
      }
    }
    // Specifically: never the word the seal shows when the samples ARE untouched.
    expect(Object.values(NO_SEAL)).not.toContain("BIT-PERFECT");
  });

  it("is a frozen module-level object, so a null seal is stable and unforgeable", () => {
    expect(Object.isFrozen(NO_SEAL)).toBe(true);
  });
});
