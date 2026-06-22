# Studio skin — implementation status

The Studio skin is implemented in the **real app** (not just the concept), as theme-owned
renderers over the shared headless hooks, selected by `data-skin="studio"` in `PlayerApp.tsx`.
Pattern: **CSS Modules** per component (collision-proof), tokens in `studio-tokens.css` +
the `[data-skin="studio"]` blocks in `neu.css`. Reference: `concepts/studio-player.html`.

## Done — every surface has a Studio renderer (light + dark)
| Surface | Component | Notes |
|---|---|---|
| Top bar | `studio/StudioTopBar.tsx` | warm header; traffic-light padding; logo = "EKO" only, consistent across themes. **Still carries skin/accent controls (interim — see below).** |
| Sidebar | `studio/StudioSidebar.tsx` | nav + music-folder + output cards |
| Library | `studio/StudioLibrary.tsx` | framed-print grid, recessed list "screens", detail, accent playing-row |
| Deck | `studio/StudioDeck.tsx` | art/meta, spectrum, EQ knobs (line indicator), **preset dropdown** (opens upward), Crossfade/ReplayGain/bit-perfect footer |
| Transport | `studio/StudioTransport.tsx` | pucks, recessed seek (under controls), **green LED meter** (driven by `useSpectrum`), 88px volume knob + VOL label |
| Queue drawer | `studio/StudioQueue.tsx` | matte slide-over (slide-in animation), framed thumbs, accent now-playing row |
| Context menu | `[data-skin="studio"] .ctxmenu` in `neu.css` | scoped override of the shared `ContextMenu` (matte panel, accent hover) |

Also: Porcelain footer lost the sample-rate/bit-perfect indicator; both skins' volume knobs are
88px (footer row grew to 116px); Porcelain knob redesigned as a proper neumorphic knob; "PRO"
removed from the Porcelain brand.

Connect/server flow is a **library empty-state**, already covered by `StudioLibrary` — there is no
separate connect modal to build.

## Native "Skins" menu — DONE
A macOS menu-bar **Skins** menu drives skin + accent + dark-mode for both skins:
`Porcelain / Studio · Accent ▸ (Orange/Violet/Blue/Teal/Graphite) · Dark Mode`, with checkmarks.

- **Rust** (`src-tauri/src/lib.rs`): the `Skins` submenu is built in `setup` from `CheckMenuItem`s
  (ids like `skin:studio`, `accent:violet`, `theme:dark`); handles are kept in a `MenuItems`
  managed state. `on_menu_event` emits a `menu-action` event with the id. A `sync_menu` command
  sets each item's `checked` from the frontend's current state.
- **Frontend** (`src/hooks/useNativeMenu.ts`, mounted in `PlayerApp`): listens for `menu-action`
  → `setSkin` / `setAccent` / `toggleTheme`; and calls `sync_menu` on mount + on any change so the
  checkmarks track the store (the source of truth, persisted to localStorage).
- The skin segment + accent picker were **removed from both headers** (`StudioTopBar`, `TopBar`);
  source + the dark/light toggle remain for quick access.

The Studio skin is now feature-complete. Verify as below; the native menu can only be checked by
clicking it in the running app.

## Verifying the skin
`cd ~/Code/llama && source ~/.nvm/nvm.sh && nvm use 22 && npm run tauri dev`, switch to **Studio**
in the header, exercise every view in **light and dark**. The frontend won't render in a plain
browser (it needs the Tauri engine).
