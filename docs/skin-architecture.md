# EKO Theming + Layout System — the plan (vetted)

> Status: **plan, vetted, awaiting sign-off. No implementation yet.** This supersedes every
> earlier version of this doc (the "chassis = React template" and the premature
> registry/manifest schema were both wrong — see §9). Vetted 2026-06-20 by three independent
> passes: a full codebase audit, an adversarial architecture critique, and prior-art research
> (VS Code, Obsidian, Radix/React-Aria, shadcn, foobar2000/Winamp, design-token standards).
> All three converged on the same core principle.

## 0. The one principle

**Split the app along the _logic ↔ presentation_ seam, never the _theme ↔ theme_ seam.**

- **Logic is shared** through headless hooks (data, state, IPC, bit-perfect derivation,
  interaction math) — written **once**.
- **No DOM is ever shared.** A theme owns **all pixels** — every leaf component and every
  layout container. (The only shared "components" are logic-free utilities like `Marquee`,
  `LocalCover`.)
- A **theme** is therefore: a **Shell** (layout) + a set of **owned presentation components** +
  **tokens**. Nothing more, nothing less.

Why this is the answer: both prior failures came from treating *a theme* as the unit of
variation. Token-only theming (shared DOM, swap colors) → Studio looked identical to Porcelain.
A parallel `StudioApp` (fork everything) → missed most of the app's features. The seam that
avoids both is logic-vs-presentation: one brain, many faces.

## 1. Why the obvious approaches fail (grounded)

- **Token-only CSS** (`[data-skin="studio"]` overriding custom properties): Porcelain and Studio
  render the **same DOM** with the **same component CSS**, so only color/material can change. A
  sidebar stays a sidebar; an album grid in beige is still the Porcelain grid. The *only* real
  difference we ever got was the EQ (knobs vs faders) — because that's the one place an actual
  **component** swaps. Confirmed live: "almost no difference."
- **Parallel app** (`StudioApp.tsx`, now deleted): re-implemented views from scratch → lost
  artists, tracks, folders, playlists, album-detail, track-lists, queue, device/RG/crossfade
  controls. Owner directive, correct: **"We never wanted a second app. This is a theme layer."**

The trap both share: **the library is the biggest surface (~80% of screen time) and has no
component-swap path** in either approach, so it never becomes distinct. The plan below makes the
library decompose like every other surface.

## 2. The architecture — three layers (theme ≠ layout ≠ logic)

```
┌─ TOKENS ────────────────────────────────────────────────────────────────┐
│  primitive → semantic → component tiers, in CSS @layer; palette = a pure  │
│  swappable value map (theme × accent × light/dark). Crosses to the mini   │
│  window. ← the ONLY thing the mini window consumes.                       │
├─ PRESENTATION (theme-owned — ALL pixels) ────────────────────────────────┤
│  ThemeProvider = { Shell, components{…}, tokens }                         │
│   • Shell        — arranges a FROZEN vocabulary of named slots (the layout)│
│   • components{}  — owned JSX for each slot, incl. anatomy swaps           │
│                    (knob↔fader, dock↔bar, card↔row). Consume hooks below. │
├─ LOGIC / ENGINE (shared — the frozen spine, written ONCE) ───────────────┤
│  Zustand stores + nativeEngine IPC + headless hooks:                      │
│   useLibrary() useTransport() useVolume() useEq() useScrub()              │
│   useSignalPath() useNowPlaying() useQueue()                              │
│  Carry ALL behavior: source-normalization, capability flags, IPC          │
│  throttling, bit-perfect derivation, scrub math. No renderer touches      │
│  useSubsonic/useLocal/nativeEngine directly.                              │
└──────────────────────────────────────────────────────────────────────────┘
```

### 2a. Logic layer — the shared spine (headless hooks)

The single most important defense against re-losing features (prior failure b): a theme component
**physically cannot** render a feed it isn't given, and **automatically** gets every feed it is.

- **`useLibrary()`** — fully **source-normalized**. Returns
  `{ albums, tracks, folders, playlists, capabilities: { tracksIndex, folders, playlists },
  detail, openAlbum(id), openArtist(name), back(), playFrom(tracks, i), albumMenuItems(card),
  trackMenuItems(tracks, i) }`. All `source === "server" | "local"` branching (async server
  fetch vs sync local, `coverArtUrl` vs `LocalCover`, "tracks index coming" capability, playlists
  server-only, folders local-only) lives **here, once**. Master/detail nav state (`detail`,
  `artist`) lifts out of the component into this layer (or a store slice). *Today this logic is
  interleaved with presentation across `LibraryView.tsx`'s 474 lines — extracting it is Phase 1.*
- **`useTransport()` / `useVolume()` / `useEq()` / `useScrub()`** — own the IPC throttling
  (volume ~50ms + pointer-lock, scrub 80ms + convergence guard) and the no-transition-on-drag
  contract. A control is then pure presentation bound to a hook.
- **`useSignalPath()`** — the single source of bit-perfect truth (today derived in *two* places,
  `TransportBar` and `SignalPath`, slightly differently). Exposes the derived `pure`/`bitPerfect`
  + the full chain (source → output device → RG → resample). **Controls bind this; the seal is a
  lit status, never a toggle.**
- **`useNowPlaying()` / `useQueue()`** — current track + queue ops.

These hooks **are** the eventual registry: a component declares which hook/feed it binds, which is
the registry *derived from real usage* rather than predicted (§9). `src/skin/feeds.ts` already
catalogs the feeds as the *intent*; the hooks are the implementation.

### 2b. Presentation layer — themes own all pixels

A **frozen, small slot vocabulary** (VS Code "Parts" lesson — a bounded contract keeps the
testing matrix and rot surface small). Proposed slots:

`shell` · `nav` · `search` · `appearance` · `library` (= `collection` + item renderers
`AlbumCard`/`TrackRow`/`ArtistRow`) · `nowPlaying` · `eq` · `transport` · `volume` · `seek` ·
`spectrum`/`meter` · `signalSeal` · `queue` · `contextMenu`.

- A **theme** supplies a component for each slot it uses + a `Shell` that arranges them.
  Porcelain `Shell` = top bar + 240px sidebar + main + bottom bar (workstation). Studio `Shell`
  = header + content + transport dock (device).
- **Anatomy swaps** (the knob-vs-fader generalization): controls with alternative anatomies
  (volume → dial/fader/slider; eq → knobs/faders; transport → bar/dock) are headless hooks whose
  visual is injected per theme (Radix `Slot`/`asChild` pattern; Figma `INSTANCE_SWAP` analogue).
- **The library decomposes too** (this is the crux the critique caught): the theme provides
  `AlbumCollection`/`AlbumCard`/`TrackRow`/`ArtistRow` — so Studio's library is *different
  components*, not the Porcelain grid in beige. No "shared styled view." Shared **hooks**, owned
  **renderers**.

### 2c. Token layer — tiered + cascade-layered

- **Three tiers:** primitive (`--warm-50`) → semantic (`--surface`, `--accent`) → component
  (`--fader-track-bg`). Re-theming repoints the *semantic* tier; component CSS stays stable.
  (DTCG / Style Dictionary discipline; `neu.css` `:root` is already tier 1–2.)
- **CSS Cascade Layers:** `@layer reset, tokens, base, theme, overrides` so a theme overrides
  base **without specificity wars** (layer order beats specificity).
- **Container queries** on controls (`container-type: inline-size`) so the same anatomy reflows
  whether dropped in a wide bar or a narrow dock — essential ahead of user-movable layouts.

## 3. Scope boundary — the mini window (do not forget this)

There are **two Tauri windows**. The `mini` (520×96, separate window) reads playback **directly
from Rust** (hidden windows get JS-timer-throttled) and gets theme/accent/skin **only via
`localStorage` + the `storage` event**. It is hand-built (`MiniWindow.tsx`) and shares no layout
with the workstation.

**Decision:** the Shell/Controls system is **main-window only**. The mini window consumes
**tokens only** (theme × accent). It is *not* a slot consumer and must never depend on a
store-driven layout it can't see. A future "Studio mini" would be its own small component — same
as today. State this as a boundary, not a gap.

## 4. Layout-as-data — later, derived, not predicted

The "build your own player" future (user mix/match/move components) is real, and the headless
foundation **enables it for free**: components already declare their hook/feed bindings → that
*is* the registry.

But: **do not build against a predicted schema.** The existing `src/skin/types.ts`
(`Registry`/`ChassisManifest`/`LayoutDoc`/`UserLayout`) was written before any renderer exists —
it will almost certainly be wrong (no master/detail nav, no async open, no per-source
capabilities). **Freeze it as documentation of intent; don't let untested types drive component
shape.** Derive the real schema *after two themes ship and you can see what actually varies.*

When it's time (Phase 5):
- A **flat node-map** keyed by id with child-id references (Craft.js style — not a deep nested
  tree; better diffs + reconciliation), over the frozen slot vocabulary.
- A `type → component` **resolver registry**.
- A root **`schemaVersion` + stepwise migrators** *before* users can save a layout (renaming a
  prop becomes a data migration across saved layouts — version first, or saved players silently
  lose controls).
- The builder UI then only **edits that document** — it adds *no new rendering path* and *no
  second copy of the engine*.

Per-theme layouts ship **hand-authored** (the `Shell` composes slots in code) long before any
builder.

## 5. Honesty + must-not-break constraints (from the audit)

1. **Bit-perfect is sacred and derived from real state.** Consolidate the two derivation sites
   into `useSignalPath()`. A `StatusLamp` binds the derived value; **never a fake `Toggle`.** The
   seal must never read "lit" when not pure.
2. **Mini window:** theme/accent/skin keep flowing via `localStorage` keys + `storage` event;
   playback keeps coming from Rust. No store-driven layout the mini can't see.
3. **Overlay titlebar:** never toggle `setDecorations` at runtime (nukes the custom header).
   Preserve `data-tauri-drag-region` on header chrome; leave room for traffic lights; replicate
   the interactive-vs-drag split.
4. **Drag/IPC perf:** hooks own throttling; **no CSS `transition` on dragged elements** (volume
   needle, seek nub, fader cap, EQ dial). Reuse store actions, never raw `invoke` in renderers.
5. **`useContextMenu` needs `{menu}` mounted once** per consuming subtree — keep one mount when
   relocating library/queue.
6. **Per-source capability flags** in `useLibrary()` so a theme can't silently lose a feature
   (the exact mechanism that re-introduces prior failure b at the component level).
7. **Retire vestigial state carefully:** `zoom`, `mainShade`, `eqVisible`, `plVisible`,
   `alwaysOnTop` are persisted (`persist.ts`) but read by nothing — remove from store + persist
   together, keep `restoreState` tolerant of old blobs.
8. **`StrictMode` double-invoke:** centralize polling in the store, not per-control.

## 6. Migration — headless-first, Porcelain pixel-identical throughout

The critique's key sequencing correction: **extract logic before touching shells.** Shell-first
locks in the DOM-sharing assumption; headless-first de-risks the crux (library distinctiveness)
before any shell effort.

| Phase | Work | Gate / verifiable by |
|------|------|----------------------|
| **1 — Headless extraction** | Extract `useLibrary()` (source-normalized + capabilities + master/detail nav) and the control hooks from today's god-components. Porcelain re-renders **identically** from them. **Zero `useSubsonic`/`useLocal`/raw IPC left in any renderer.** | Porcelain **pixel-identical**; clean separation — **Gate 2** |
| **2 — Token tiers + layers** | Restructure `neu.css` → primitive/semantic/component tiers in `@layer`; container-query the controls | Porcelain identical |
| **3 — Theme contract** | Introduce `ThemeProvider`; express Porcelain as `{ PorcelainShell, porcelainComponents, tokens }`; freeze the slot vocabulary | Porcelain identical |
| **4 — Studio** | Build Studio's `Shell` + **owned renderers (incl. the library: distinct cards/rows, not just knobs)** + tokens, over the shared hooks | Studio matches the approved concepts |
| **5 — Layout-as-data + builder** *(later)* | Derive the schema from the two shipped themes; flat node-map + resolver + `schemaVersion`; the builder edits the document | — |

Porcelain is untouched-in-appearance through 1–3; Studio is additive in 4; nothing touches the
**audio engine or bit-perfect path** in any phase.

## 7. Two go/no-go gates — BEFORE committing build effort

- **Gate 1 — taste (concepts-first, the LIBRARY not the transport):** a static, browser-openable
  Studio concept covering the **full library** — album grid, **album detail, track list, artist
  list** — in Studio styling, plus now-playing. If the *library* doesn't read as meaningfully
  distinct from Porcelain (not just knobs-vs-faders), the premise is false — **stop**, for the
  cost of an afternoon in `concepts/`, before any extraction. *(This is also the answer to the
  earlier "it's missing things" — design the whole surface, not the happy path.)*
- **Gate 2 — architecture:** `useLibrary()` extracted and Porcelain rendering **pixel-identical**
  from it, with **zero** `useSubsonic`/`useLocal` references in any renderer. Clean ⇒ Studio is a
  styling+layout exercise over a proven brain. Messy ⇒ the boundary is wrong and no shell saves
  it.

## 8. Open decisions for sign-off

1. **Endorse the seam:** share **logic via hooks**, never DOM; theme owns all pixels; **delete
   the "shared styled view" idea** (the library decomposes into shared hooks + theme-owned
   renderers). *(Strong rec; this is the vetted core.)*
2. **Freeze `src/skin/` schema as docs**, build Studio against a plain `ThemeProvider` context,
   and derive the layout-as-data schema only after two themes exist. *(Avoids the
   over-engineering trap the critique flagged.)*
3. **Mini window = tokens only**, main window owns Shell/Controls. *(Boundary, not gap.)*
4. **Sequence headless-first** (extract `useLibrary()` + control hooks before shells), gated by
   Gate 2.
5. **Concepts-first on the library** (Gate 1) before any code.

## 9. What this corrects from earlier drafts

- "Built-in chassis = full React templates" → **wrong** (that's the parallel-app trap). Themes
  are presentation over shared hooks, not apps.
- "Registry / ChassisManifest / LayoutDoc schema, built first" → **premature**. Freeze as intent;
  derive after two themes. The schema with no renderer is "the over-engineering trap already
  sprung."
- "Shared content views, token-styled" → **re-introduces the token-only failure on the library.**
  Replaced by: shared hooks, theme-owned renderers; no shared DOM.
- Palette/Chassis/Layout split → kept in spirit (theme ≠ layout ≠ logic) but re-grounded: Palette
  = tokens, "Chassis" = Shell + owned components (React, headless-backed), Layout = a *later,
  derived* data document.

## 10. Prior art (why we trust the shape)

- **Logic/presentation seam, Slot injection:** Radix Primitives (`asChild`/`Slot`), React Aria
  Components, shadcn/ui "own the code" registry. Logic shared via hooks; markup never shared.
- **Theme ≠ layout, frozen Parts vocabulary:** VS Code (color themes vs workbench Parts /
  contribution points), Obsidian (CSS themes vs `workspace.json` layout tree).
- **Layout as a serializable tree, owned separately → saved layouts ~free:** Obsidian Workspaces,
  foobar2000 DUI `.fth`, dockview `toJSON/fromJSON`.
- **Tiered tokens:** DTCG format + Style Dictionary.
- **CSS:** Cascade Layers (`@layer`) for override order, container queries for placement-agnostic
  controls.
- **Layout-as-data pitfalls:** schema-versioning-first, the inner-platform effect, structural
  skin rot (Winamp modern ≈ 6 months vs classic ≈ minutes), flat node-maps over deep trees
  (Craft.js).
