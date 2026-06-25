import { Spectrum } from "./Spectrum";

/**
 * MiniEq — an alternative to the segmented LED <Meter> for the transport "meter" slot: a small
 * recessed dot-matrix spectrum (the "mini EQ" from the UI kits, e.g. concepts/kit-porcelain). Shares
 * the live <Spectrum> renderer in dot mode; the recessed well (.minieq) is token-skinned in neu.css
 * so it re-tints per skin. Shared/free — registered as the "miniEq" meter-slot variant for all presets.
 */
export function MiniEq() {
  return (
    <div className="minieq" aria-hidden="true">
      <Spectrum dots bands={13} bargap={2.5} className="minieq-cv" />
    </div>
  );
}
