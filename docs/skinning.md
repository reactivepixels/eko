# Skinning EKO

EKO's look is driven entirely by **CSS custom properties (design tokens)** declared in
`src/player/neu.css`. Nothing in the component tree hard-codes a colour or shadow — every
surface, text colour, shadow and accent is a `var(--token)`. That makes the player skinnable
along three independent axes:

| Axis | Attribute | State (persisted) | Status |
|------|-----------|-------------------|--------|
| **Theme** — light vs dark | `data-theme="light" \| "dark"` | `useUiStore.theme` → `localStorage["eko.theme"]` | shipped |
| **Accent** — the highlight colour | `data-accent="orange \| violet \| blue \| teal \| graphite"` | `useUiStore.accent` → `localStorage["eko.accent"]` | shipped |
| **Skin** — a full token bundle | `data-skin="…"` | _planned_ | proposed |

All three are applied as attributes on the root element (`.app` in `PlayerApp.tsx`, `.capp`
in `MiniWindow.tsx`) and resolved by attribute-selector blocks in `neu.css`. They compose:
a skin sets the base tokens, the theme overrides the light/dark subset, and the accent
overrides just the accent subset.

> **The golden rule:** a skin only ever changes **token values**. It never adds selectors,
> markup, or behaviour. This keeps skins safe to share, keeps EKO's identity intact, and
> keeps the **bit-perfect seal honest** (see [Identity & the seal](#identity--the-seal)).

---

## How it works today

`neu.css` declares the defaults on `:root` (the light "Porcelain" theme), then overrides the
relevant tokens under attribute selectors:

```css
:root {
  --bg: #e9e7e0;
  --ink: #2c2b27;
  --accent: #ef6a1e;       /* EKO orange */
  --accent-2: #ff8c42;
  --accent-ink: #fffaf6;
  /* …shadows, screen, spectrum, etc. */
}

/* dark "Graphite" — overrides surfaces, ink and the neumorphic shadow recipe */
[data-theme="dark"] {
  --bg: #23262c;
  --ink: #eceef0;
  --out: 5px 5px 13px var(--dark), -3px -3px 9px var(--light);
  /* … */
}

/* accent presets — override ONLY the accent tokens, so they work in light + dark */
[data-accent="violet"] { --accent: #6a5cf0; --accent-2: #8d7dff; --accent-ink: #fbfaff; }
```

The React side just reflects store state onto the root and persists it:

```tsx
// PlayerApp.tsx
<div className="app" data-theme={theme} data-accent={accent}>
```

```ts
// useUiStore.ts — accent is persisted exactly like theme
setAccent: (accent) =>
  set(() => { localStorage.setItem("eko.accent", accent); return { accent }; }),
```

The mini window is a separate WebView, so it re-reads `localStorage` on its poll and on the
`storage` event (it can't use the store directly — hidden windows get JS-timer-throttled).

---

## Add an accent preset

1. **Add the tokens** in `neu.css` (next to the other `[data-accent]` blocks):

   ```css
   [data-accent="rose"] { --accent: #e0457e; --accent-2: #ee6c9a; --accent-ink: #fff5f8; }
   ```

   You only need `--accent` (the colour), `--accent-2` (a lighter shade for gradients) and
   `--accent-ink` (legible text **on** an accent fill — keep contrast ≥ 4.5:1).

2. **Register it** in `ACCENTS` in `src/store/useUiStore.ts` (id + label + picker swatch) and
   add the id to the `Accent` union:

   ```ts
   export type Accent = "orange" | "violet" | "blue" | "teal" | "graphite" | "rose";
   export const ACCENTS = [ /* … */ { id: "rose", label: "Rose", swatch: "#e0457e" } ];
   ```

That's it — the picker in `TopBar.tsx` renders from `ACCENTS`, and the choice persists. No
component changes.

---

## Token reference

The skinnable contract. Names are stable; values are yours to set per skin.

### Surfaces & structure
| Token | Role |
|-------|------|
| `--bg`, `--bg2` | base panel / raised panel |
| `--light`, `--dark` | the two neumorphic shadow colours (highlight / shadow) |
| `--line` | hairline / etched divider |
| `--art-bg` | placeholder cover background |
| `--hover` | subtle row/control hover tint |

### Text
| Token | Role |
|-------|------|
| `--ink` | primary text |
| `--ink-2` | secondary text |
| `--ink-3` | tertiary / disabled |

### Accent (the one highlight colour)
| Token | Role |
|-------|------|
| `--accent` | the live/active colour (LEDs, primary fills, active states) |
| `--accent-2` | lighter shade for gradients |
| `--accent-ink` | text on top of an accent fill |

### Shadow recipe (neumorphism)
| Token | Role |
|-------|------|
| `--out`, `--out-sm`, `--out-xs` | raised elements (3 depths) |
| `--in`, `--in-deep` | recessed elements |
| `--bevel` | the thin inner bevel on raised controls |
| `--lift`, `--lift-sm` | floating cards / drop shadows |

### Screens & meters
| Token | Role |
|-------|------|
| `--screen` | the dark VFD/readout glass |
| `--g`, `--g2`, `--a`, `--r` | meter green / dim-green / amber / red |
| `--spec-rgb` | spectrum bar colour (R,G,B triplet, used translucent) |

### Misc
`--font`, `--mono` (type), `--ec` (the shared easing curve).

> When adding a token, ask: *is it a value a skin would reasonably want to change?* If yes,
> it belongs here. If it's structural (a `grid-template`, a size), it does **not** — skins
> change appearance, not layout.

---

## Built-in skins _(proposed)_

A full skin is the same mechanism as the dark theme: a named block that re-declares the base
tokens, optionally theme-aware.

```css
[data-skin="studio"] {
  --bg: #f3f1ec; --ink: #3b3933; /* …the full token set… */
}
[data-skin="studio"][data-theme="dark"] {
  --bg: #21242a; --ink: #e9e7e2; /* …dark overrides… */
}
```

Wiring mirrors the accent work: a `skin: string` field in `useUiStore` (persisted), a
`data-skin={skin}` on the roots, and a picker in Settings. The **Studio** concept
(`concepts/skin-studio.html` + `…-dark.html`) is the reference design for the first such skin.

---

## User-authored skins _(proposed)_

So anyone can ship a skin without touching the app:

- **Format — a declarative JSON manifest** (not raw CSS, for safety):

  ```jsonc
  {
    "name": "Midnight Brass",
    "author": "you",
    "version": "1.0.0",
    "base": "dark",                 // light | dark — which theme it extends
    "tokens": {                     // ONLY known --tokens, validated on load
      "--bg": "#0e0f12",
      "--ink": "#eae7df",
      "--accent": "#c8a24a"
    },
    "accents": { "brass": "#c8a24a" } // optional extra accent presets
  }
  ```

- **Install** — drop the file in the skins folder
  (`~/Library/Application Support/<bundle-id>/skins/*.json`) or use **Settings → Import skin…**
  (Tauri `dialog` + `fs`).

- **Loader** — read → **validate every key against the token allow-list** and every value as a
  CSS colour/shadow → inject as a scoped `<style>` (`[data-skin="user:<id>"] { … }`). Reject
  unknown keys and anything that isn't a token value. **Never `eval` CSS or accept selectors.**

This keeps shared skins safe (no arbitrary CSS/JS), and means a skin is just a small,
diff-able, hostable text file.

---

## Identity & the seal

Some things are **not** skinnable, by design:

- The **four-bar EKO wordmark**, the **signal path**, and the **bit-perfect seal** are always
  present and always reflect *real* state. A skin can recolour them via tokens but cannot
  remove, relabel, or fake them.
- The **bit-perfect seal must never be styled to read "lit" when the path isn't bit-perfect.**
  It is bound to the engine's actual state (unity volume + flat EQ + matched device rate).
- Keep **contrast legible** — `--ink` on `--bg`, and `--accent-ink` on `--accent`, should meet
  WCAG AA. The picker/Settings should warn (not block) on low-contrast custom skins.

Skinning is **UI-only** — it never touches the audio path, so it's safe to iterate without
ear-testing (unlike gapless / ReplayGain).

---

## Files

- `src/player/neu.css` — all tokens + theme/accent blocks.
- `src/store/useUiStore.ts` — `theme`, `accent` state + `ACCENTS` presets (persisted).
- `src/player/TopBar.tsx` — theme toggle + accent picker.
- `src/player/PlayerApp.tsx`, `MiniWindow.tsx` — apply `data-theme` / `data-accent`.
- `concepts/skin-studio.html`, `concepts/skin-studio-dark.html` — the Studio skin concept
  (light + dark), the reference for the first built-in `data-skin`.
