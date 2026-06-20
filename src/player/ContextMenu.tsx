import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** One entry in a context menu. A `separator` draws a divider; otherwise it's an action. */
export type MenuItem =
  | { separator: true }
  | { label: string; onSelect: () => void; danger?: boolean; disabled?: boolean };

interface MenuState {
  x: number;
  y: number;
  items: MenuItem[];
}

/**
 * Right-click context menu, shared across the library and queue. Returns `open` — a
 * factory that builds an `onContextMenu` handler for a given item list — and `menu`,
 * the floating element to drop once into the rendering view. The menu closes on outside
 * click, Escape, scroll, blur, or resize, and clamps itself inside the viewport.
 */
export function useContextMenu() {
  const [state, setState] = useState<MenuState | null>(null);
  const ref = useRef<HTMLDivElement>(null);

  const open = (items: MenuItem[]) => (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setState({ x: e.clientX, y: e.clientY, items });
  };

  useEffect(() => {
    if (!state) return;
    const close = () => setState(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("keydown", onKey);
    };
  }, [state]);

  // Keep the menu fully on-screen — flip/clamp against the right and bottom edges.
  useLayoutEffect(() => {
    if (!state || !ref.current) return;
    const el = ref.current;
    const { width, height } = el.getBoundingClientRect();
    const pad = 8;
    let { x, y } = state;
    if (x + width + pad > window.innerWidth) x = window.innerWidth - width - pad;
    if (y + height + pad > window.innerHeight) y = window.innerHeight - height - pad;
    el.style.left = `${Math.max(pad, x)}px`;
    el.style.top = `${Math.max(pad, y)}px`;
  }, [state]);

  const menu = state ? (
    <div
      ref={ref}
      className="ctxmenu"
      style={{ left: state.x, top: state.y }}
      onPointerDown={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      {state.items.map((item, i) =>
        "separator" in item ? (
          <div key={i} className="ctx-sep" />
        ) : (
          <button
            key={i}
            className={`ctx-item${item.danger ? " danger" : ""}`}
            disabled={item.disabled}
            onClick={() => {
              setState(null);
              item.onSelect();
            }}
          >
            {item.label}
          </button>
        ),
      )}
    </div>
  ) : null;

  return { open, menu };
}
