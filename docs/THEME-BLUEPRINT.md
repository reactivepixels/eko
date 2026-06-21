# EKO Theme Blueprint

The repeatable playbook for building a theme — distilled from building **Studio**, so every
future theme is fast and right the first time. Pairs with `docs/skin-architecture.md` (the
*technical* seam: shared logic ↔ theme-owned pixels). This doc is the *process + design law +
surface checklist*.

## 1. The one law

**A theme is a complete material + component transformation, NOT a recolour.** If a new theme
shares the same DOM and CSS as another and only repoints colour tokens, it will look like the
other theme. Every surface must be re-materialised.

- Keep the **feature set** of the reference theme (Porcelain): every screen, every nav path,
  every control. A theme never *removes* features to match a simplified mockup.
- Transform the **material** of every surface: shape, depth, shadow, recess, typography,
  controls' anatomy.
- **Structure** is a judgment call per theme — keep the reference layout where it serves, change
  it where the aesthetic demands (e.g. a transport *dock* vs a transport *bar*). Primarily visual.

## 2. Lessons paid for (do not relearn these)

1. **Token-only theming fails.** `[data-skin="x"]` overriding CSS variables on shared DOM can only
   change colour/shadow — the album grid in beige is still the same grid. Confirmed twice.
2. **Reskinning one surface is worse than nothing.** Studio's library got a renderer while the top
   bar, sidebar, transport, and deck stayed Porcelain → the app still read as Porcelain. **A theme
   is only "done" when EVERY surface is transformed.** Partial = looks unchanged.
3. **Concept-first, always.** Do not iterate the live app toward an unapproved look — it reads as
   churn and frustrates. Lock the aesthetic in a **complete static HTML concept** (full feature
   set), get explicit approval, *then* implement in one coherent pass.
4. **The concept guides aesthetic, not literal layout.** Treat the inspiration concept as the
   material/feel reference; the shipped player must still be fully featured.
5. **Bit-perfect is sacred.** No theme work touches the audio engine or the bit-perfect path.

## 3. The process (every theme)

1. **Aesthetic lock (Gate 1).** One browser-openable HTML concept covering the **full** feature
   set in the new material — every screen + up-next + context menu + connect/empty states + light
   & dark. Approved before any app code. (Studio: `concepts/studio-player.html`, guided by
   `skin-studio.html` + `skin-studio-library.html`.)
2. **Confirm the shared brain exists.** Phase 1 of `skin-architecture.md` is done: all logic lives
   in headless hooks (`useLibrary`, `useTransport`, `useSignalPath`, `useEq`, `useQueue`,
   `useNowPlaying`, `useVolume`, `useScrub`, `useMusicSource`, `useConnect`). A theme consumes these
   and renders pixels only.
3. **Build theme-owned renderers + a theme stylesheet** for **every** surface in §4, over those
   hooks. Own all pixels; share zero DOM with other themes.
4. **Mount via the theme switch** (`data-skin`): the active theme supplies the component for each
   surface (a Shell composing them). Porcelain stays pixel-identical and untouched.
5. **Verify surface-by-surface against the concept.** Not "close" — matching.

## 4. The surface checklist (a theme is done when ALL are transformed)

| # | Surface | Real component(s) | Hook(s) | Studio treatment (from the concept) |
|---|---------|-------------------|---------|--------------------------------------|
| 1 | Header / top bar | `TopBar` | `useMusicSource`, ui | matte bevel header; mono brand/sub; segmented browse/now; recessed search well; source/skin/theme/accent as device controls |
| 2 | Sidebar / nav | `Sidebar` | `useMusicSource` | mono nav headers; inset-well active item; matte folder + output cards |
| 3 | Album grid | `LibraryView`→`StudioLibrary` | `useLibrary` | framed-print covers (inset face mat + warm drop); mono catalogue label; now-playing badge |
| 4 | Album detail | (same) | `useLibrary` | big framed cover; mono meta; **recessed track "screen"**; pill actions |
| 5 | Artists / Tracks / Folders / Playlists | (same) | `useLibrary` | device list rows in recessed screens; mono sub-labels |
| 6 | Now-playing deck | `DeckView` | `useNowPlaying`,`useEq`,`useSignalPath` | the console: **big flat-top volume knob**, recessed **spectrum screen**, EQ **knobs** + preset pills + slide toggles + LED meter |
| 7 | Signal path / seal | `SignalPath` | `useSignalPath` | mono signal chain + bit-perfect lock lamp |
| 8 | Transport | `TransportBar` | `useTransport`,`useVolume`,`useScrub` | the **dock**: flat-top transport pucks (press-in), seek slot well, dock volume knob, bit-perfect lamp, mini meter |
| 9 | Up-next / queue | `QueuePanel` | `useQueue` | matte slide-over drawer; framed thumbs; mono meta |
| 10 | Context menu | `ContextMenu` | (items from hooks) | matte rounded menu; mono items; accent hover |
| 11 | Connect / empty / scanning | `ConnectPanel`, empties | `useConnect`,`useMusicSource` | matte modal + mono empty states |
| 12 | Spectrum / meters | `Spectrum` | `useSpectrum` | warm LED-segment device screen |
| 13 | Light **and** dark | tokens | — | warm matte (light) + warm-dark device (dark) |

## 5. The Studio material (reference values)

Source of truth: `concepts/skin-studio.html` (device controls) + `concepts/studio-player.html`
(full app). Core tokens:

- Faces `--face:#f3f1ec`, recessed `--face-lo:#e7e3da`, device bg `#e4e0d8`.
- **Warm** shadow rgb `--sh:62,58,50` (never black).
- Ink `#3b3933` / `#6d6a61`; mono micro-labels `--label:#a4a096` (`SF Mono`, uppercase, tracked).
- Recess `--in: inset 2px 2px 5px rgba(sh,.16), inset -2px -2px 5px rgba(255,255,255,.92)`.
- Raised cap `--raise: 0 2px 4px rgba(sh,.16), 0 12px 22px -10px rgba(sh,.28), inset 0 1px 1px #fff, 0 0 0 1px rgba(255,255,255,.55)`.
- Knob = convex multi-layer body + flat radial top + accent gauge arc + indicator (line for EQ,
  dot for volume). Buttons = flat-top pucks that press *into* the panel. Accent drives gauges,
  fills, play halo, lamps.

## 6. For the next theme

Repeat §3 with a new material. The hooks, the surface checklist (§4), and the gates don't change —
only the concept and the theme-owned renderers/stylesheet do. The cost of theme #2+ is mostly the
concept (Gate 1) + the per-surface CSS; the brain is already shared.
