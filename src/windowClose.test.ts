import { describe, it, expect } from "vitest";
// `?raw` (declared by `vite/client`) rather than `node:fs`: this tsconfig carries no Node
// types — same reasoning as `pro/offlineMenu.test.ts`.
import appSource from "./App.tsx?raw";
import capabilitySource from "../src-tauri/crates/eko-tauri/capabilities/default.json?raw";

/**
 * Window-close guard (public issue #4: "click close button but not work").
 *
 * Tauri's Rust side PREVENTS the native close as soon as the webview has a
 * `tauri://close-requested` listener (tauri's `manager/window.rs`: `has_js_listener` →
 * `api.prevent_close()`). From then on the only thing that actually closes the window is the
 * `window.destroy()` that `onCloseRequested`'s JS wrapper issues once the handler resolves —
 * and `destroy` is an IPC command, so it needs `core:window:allow-destroy` in the capability
 * file. That is NOT part of `core:window:default` (which is all read-only getters), so it has
 * to be listed explicitly.
 *
 * Miss either half and the red traffic light silently does nothing: prevent_close fires, and
 * then destroy is refused by the ACL (or skipped because the handler rejected), so the window
 * stays up. Both halves are asserted here because that is precisely what shipped in 0.4.33,
 * when the flush-state-on-quit handler was added without the matching permission.
 */

const permissions: string[] = JSON.parse(capabilitySource).permissions;

// The `onCloseRequested(...)` call, from the callback open-brace to its closing `});`.
const closeHandler = (() => {
  const at = appSource.indexOf("onCloseRequested");
  if (at < 0) return null;
  const rest = appSource.slice(at);
  return rest.slice(0, rest.indexOf("});") + 3);
})();

describe("window close", () => {
  it("registers a close-requested listener (the reason destroy is needed at all)", () => {
    expect(closeHandler).not.toBeNull();
  });

  it("permits window.destroy whenever a close-requested listener is registered", () => {
    if (!closeHandler) return; // no listener → native close still works, no permission needed
    expect(permissions).toContain("core:window:allow-destroy");
  });

  it("guards the handler body — a throw would skip destroy and wedge the window open", () => {
    if (!closeHandler) return;
    expect(closeHandler).toMatch(/try\s*\{/);
  });
});
