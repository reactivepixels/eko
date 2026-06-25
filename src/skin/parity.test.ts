import { describe, it, expect } from "vitest";
import { DEFAULT_SLOT_VARIANT, presetSlots, GENERIC_SHELL_PRESETS } from "./presets";
import type { Role } from "./vocabulary";

/**
 * Theme parity guard (component-variants-plan P4).
 *
 * The rule: "a skin changes material only — never the feature set." Every theme runs on ONE shared
 * shell that renders the full control inventory as slots, and every preset must fill EVERY inventory
 * slot EXPLICITLY (not by silently falling back to the Porcelain free default — that would make a
 * theme inherit Porcelain's control for a slot it forgot to wire). This is pure data over presets.ts
 * so it needs no registry/store/DOM; adding a theme to GENERIC_SHELL_PRESETS auto-enrolls it here.
 *
 * Registry integrity (each id resolves to a registered variant of the right slot/tier) is enforced
 * at load by the Slot resolver + the duplicate-id throw + typecheck on the registration arrays.
 */

// The control-slot inventory = the free baseline's slots. Every theme must fill all of them.
const INVENTORY = Object.keys(DEFAULT_SLOT_VARIANT) as Role[];

// All themes on the shared shell. New themes join here → automatically parity-checked.
const SKINS = [...GENERIC_SHELL_PRESETS];

describe("theme feature parity", () => {
  it("the control inventory is the expected six slots (guards an empty/eroded inventory)", () => {
    expect([...INVENTORY].sort()).toEqual(
      ["eq", "meter", "seek", "spectrum", "transport", "volume"].sort(),
    );
  });

  it.each(SKINS)("preset '%s' fills every inventory slot explicitly", (skin) => {
    const slots = presetSlots(skin);
    for (const slot of INVENTORY) {
      const id = slots[slot];
      expect(id, `${skin} does not define a variant for the "${slot}" slot`).toBeTruthy();
      expect(typeof id).toBe("string");
    }
  });

  it.each(SKINS)("preset '%s' maps only known inventory slots (no typo'd/orphan slot)", (skin) => {
    for (const slot of Object.keys(presetSlots(skin))) {
      expect(INVENTORY, `${skin} maps unknown slot "${slot}"`).toContain(slot);
    }
  });

  it("every theme uses the single shared meter (the meter is one unified design)", () => {
    for (const skin of SKINS) {
      expect(presetSlots(skin).meter, `${skin} should use the shared "meter" variant`).toBe("meter");
    }
    expect(DEFAULT_SLOT_VARIANT.meter).toBe("meter");
  });
});
