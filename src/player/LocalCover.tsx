import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// Lazily-fetched embedded covers, cached by file path so each is read once.
const cache = new Map<string, string | null>();

/** Renders the embedded cover art for a local file (or nothing if it has none). */
export function LocalCover({ path, className }: { path?: string; className?: string }) {
  const [src, setSrc] = useState<string | null>(() =>
    path && cache.has(path) ? cache.get(path)! : null,
  );

  useEffect(() => {
    if (!path) {
      setSrc(null);
      return;
    }
    if (cache.has(path)) {
      setSrc(cache.get(path) ?? null);
      return;
    }
    let alive = true;
    invoke<string | null>("read_cover", { path })
      .then((d) => {
        cache.set(path, d ?? null);
        if (alive) setSrc(d ?? null);
      })
      .catch(() => {
        cache.set(path, null);
      });
    return () => {
      alive = false;
    };
  }, [path]);

  return src ? <img src={src} className={className} alt="" /> : null;
}
