/**
 * AccentPicker — free accent-color swatch row (orange/violet/blue/teal/graphite/cyan).
 *
 * Accent color is a free feature (unlike alternate skins, which are Pro-gated).
 * This component lives in src/player/ (not behind @pro) and is rendered in TopBar
 * in both free and Pro builds. It reuses the `.accent-pick` / `.acc-sw` CSS already
 * defined in neu.css.
 */
import type { CSSProperties } from "react";
import { ACCENTS, useUiStore } from "../store/useUiStore";

export function AccentPicker() {
  const accent = useUiStore((s) => s.accent);
  const setAccent = useUiStore((s) => s.setAccent);

  return (
    <div className="accent-pick" role="group" aria-label="Accent color">
      {ACCENTS.map((a) => (
        <button
          key={a.id}
          type="button"
          className={`acc-sw${accent === a.id ? " on" : ""}`}
          style={{ "--c": a.swatch } as CSSProperties}
          title={a.label}
          aria-label={a.label}
          aria-pressed={accent === a.id}
          onClick={() => setAccent(a.id)}
        />
      ))}
    </div>
  );
}
