/**
 * `CanvasRenderingContext2D.roundRect` polyfill for older WebKit.
 *
 * The native method only shipped in Safari 16.4 (macOS 13.3, March 2023). Tauri renders through
 * the system WKWebView, and EKO advertises support back to macOS 10.13 (`LSMinimumSystemVersion`),
 * so on an older Mac the method is missing — the spectrum canvas (`Spectrum.tsx`) throws on its
 * first frame and the display never draws (the audio path is unaffected, so it looks like "the EQ
 * works but not visually").
 *
 * Feature-detected: a no-op on any WebView that already has the native method. Only the single
 * `number` radius form EKO uses is meaningful; other spec forms fall back to no rounding.
 */
export function installRoundRectPolyfill(): void {
  if (typeof CanvasRenderingContext2D === "undefined") return;
  const proto = CanvasRenderingContext2D.prototype;
  if (typeof proto.roundRect === "function") return;

  proto.roundRect = function (
    this: CanvasRenderingContext2D,
    x: number,
    y: number,
    w: number,
    h: number,
    radii: number | DOMPointInit | (number | DOMPointInit)[] = 0,
  ): void {
    const r = Math.max(
      0,
      Math.min(typeof radii === "number" ? radii : 0, Math.abs(w) / 2, Math.abs(h) / 2),
    );
    this.moveTo(x + r, y);
    this.arcTo(x + w, y, x + w, y + h, r);
    this.arcTo(x + w, y + h, x, y + h, r);
    this.arcTo(x, y + h, x, y, r);
    this.arcTo(x, y, x + w, y, r);
    this.closePath();
  };
}
