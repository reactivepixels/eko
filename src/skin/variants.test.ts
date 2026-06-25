import { describe, it, expect } from "vitest";
import { registerVariant, getVariant, variantsForSlot, listVariants } from "./variants";

const Noop = () => null;

describe("variant registry", () => {
  it("registers and resolves a variant by id and by slot", () => {
    registerVariant({ id: "testVol", slot: "volume", label: "Test · Vol", tier: "free", Component: Noop });
    expect(getVariant("testVol")?.label).toBe("Test · Vol");
    expect(variantsForSlot("volume").some((v) => v.id === "testVol")).toBe(true);
    expect(listVariants().some((v) => v.id === "testVol")).toBe(true);
  });

  it("returns undefined for unknown/empty ids", () => {
    expect(getVariant("nope")).toBeUndefined();
    expect(getVariant(undefined)).toBeUndefined();
  });

  it("throws on a duplicate id (a rename/wiring mistake fails loudly)", () => {
    registerVariant({ id: "dupe", slot: "eq", label: "A", tier: "free", Component: Noop });
    expect(() => registerVariant({ id: "dupe", slot: "eq", label: "B", tier: "free", Component: Noop })).toThrow();
  });
});
