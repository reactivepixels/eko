import { useEffect, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import { Marquee } from "./Marquee";
import { LocalCover } from "./LocalCover";
import { nativeEngine, type EngineStatus, type NowPlaying } from "../audio/nativeEngine";
import { ACCENTS, SKINS, type Accent, type Skin } from "../store/useUiStore";
import "./neu.css";

const NP0: NowPlaying = {
  title: "EKO",
  artist: "Pick an album",
  coverUrl: "",
  coverPath: "",
  theme: "light",
  index: -1,
  total: 0,
};
const ST0: EngineStatus = {
  playing: false,
  posMs: 0,
  durMs: 0,
  rate: 0,
  channels: 0,
  device: "",
  srcRate: 0,
  devRate: 0,
  bits: 0,
  codec: "",
  seg: 0,
};

/** The light/dark mode is shared across windows via localStorage (the engine's NowPlaying
 *  theme is only set once a track plays, so the mini can't rely on it). */
function readTheme(): "light" | "dark" {
  try {
    return localStorage.getItem("eko.theme") === "dark" ? "dark" : "light";
  } catch {
    return "light";
  }
}

/** Accent is shared across windows via localStorage, same as the theme. */
function readAccent(): Accent {
  try {
    const v = localStorage.getItem("eko.accent") as Accent | null;
    return v && ACCENTS.some((a) => a.id === v) ? v : "orange";
  } catch {
    return "orange";
  }
}

/** Skin is shared across windows via localStorage, same as the theme. */
function readSkin(): Skin {
  try {
    const v = localStorage.getItem("eko.skin") as Skin | null;
    return v && SKINS.some((s) => s.id === v) ? v : "porcelain";
  } catch {
    return "porcelain";
  }
}

/**
 * Frameless always-on-top mini player. Reads playback state DIRECTLY from the Rust
 * engine (so it stays live even when the main window is hidden and its JS timers are
 * throttled by macOS), and drives pause/resume/seek straight into the engine. Only
 * next/prev/expand are sent to the main window, which owns the playlist.
 */
export function MiniWindow() {
  const [np, setNp] = useState<NowPlaying>(NP0);
  const [st, setSt] = useState<EngineStatus>(ST0);
  const [theme, setTheme] = useState<"light" | "dark">(readTheme);
  const [accent, setAccent] = useState<Accent>(readAccent);
  const [skin, setSkin] = useState<Skin>(readSkin);
  // Optimistic scrub position so the bar tracks the drag without the poll fighting it.
  const [scrub, setScrub] = useState<number | null>(null);

  useEffect(() => {
    let alive = true;
    const poll = async () => {
      const [s, n] = await Promise.all([
        nativeEngine.status().catch(() => null),
        nativeEngine.nowPlaying().catch(() => null),
      ]);
      if (!alive) return;
      if (s) setSt(s);
      if (n) setNp(n);
      setTheme(readTheme()); // keep the mini's mode in sync with the main window
      setAccent(readAccent());
      setSkin(readSkin());
    };
    void poll();
    const t = setInterval(poll, 300);
    const onStorage = () => {
      setTheme(readTheme());
      setAccent(readAccent());
      setSkin(readSkin());
    };
    window.addEventListener("storage", onStorage);
    return () => {
      alive = false;
      clearInterval(t);
      window.removeEventListener("storage", onStorage);
    };
  }, []);

  const cmd = (action: string) => {
    void emit("eko:cmd", { action });
  };
  const durS = st.durMs / 1000;
  const prog =
    scrub != null ? (durS > 0 ? scrub / durS : 0) : st.durMs > 0 ? st.posMs / st.durMs : 0;

  const secsAt = (clientX: number, el: Element) => {
    const r = el.getBoundingClientRect();
    return Math.min(1, Math.max(0, (clientX - r.left) / r.width)) * durS;
  };
  const onDown = (e: React.PointerEvent) => {
    if (durS <= 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setScrub(secsAt(e.clientX, e.currentTarget));
  };
  const onMove = (e: React.PointerEvent) => {
    if (scrub == null || !(e.buttons & 1)) return;
    setScrub(secsAt(e.clientX, e.currentTarget));
  };
  const onUp = (e: React.PointerEvent) => {
    if (scrub == null) return;
    const s = secsAt(e.clientX, e.currentTarget);
    void nativeEngine.seek(s);
    setSt((p) => ({ ...p, posMs: s * 1000 }));
    setScrub(null);
  };

  const toggle = () => {
    if (st.playing) {
      void nativeEngine.pause();
      setSt((p) => ({ ...p, playing: false }));
    } else {
      void nativeEngine.resume();
      setSt((p) => ({ ...p, playing: true }));
    }
  };

  const hasTrack = np.index >= 0;

  return (
    <div className="capp mini-rounded" data-theme={theme} data-accent={accent} data-skin={skin}>
      <div className="cbar nolights" data-tauri-drag-region>
        <div className="cart" data-tauri-drag-region>
          {np.coverUrl ? (
            <img src={np.coverUrl} alt="" />
          ) : np.coverPath ? (
            <LocalCover path={np.coverPath} />
          ) : null}
        </div>
        <div className="cmid" data-tauri-drag-region>
          <div className="ctt">
            <Marquee text={np.title} />
          </div>
          <div className="car">{np.artist}</div>
          <div
            className="ctrack"
            data-tauri-drag-region="false"
            onPointerDown={onDown}
            onPointerMove={onMove}
            onPointerUp={onUp}
            onPointerCancel={onUp}
          >
            <div className="cfill" style={{ width: `${prog * 100}%` }} />
            <div className="cknob" style={{ left: `${prog * 100}%` }} />
          </div>
        </div>
        <div className="cctrls" data-tauri-drag-region="false">
          <div className="tbtn" title="Previous" onClick={() => cmd("prev")}>
            <svg viewBox="0 0 24 24">
              <path d="M7 6h2v12H7zM19 6v12l-9-6z" />
            </svg>
          </div>
          <div className="tbtn play" title="Play / Pause" onClick={toggle}>
            {st.playing ? (
              <svg viewBox="0 0 24 24">
                <path d="M7 5h3.5v14H7zM13.5 5H17v14h-3.5z" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24">
                <path d="M8 5v14l11-7z" />
              </svg>
            )}
          </div>
          <div className="tbtn" title="Next" onClick={() => cmd("next")}>
            <svg viewBox="0 0 24 24">
              <path d="M15 6h2v12h-2zM5 6l9 6-9 6z" />
            </svg>
          </div>
        </div>
        <div
          className="tbtn cexp"
          data-tauri-drag-region="false"
          title="Expand"
          onClick={() => cmd("expand")}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M4 14v6h6M20 10V4h-6M20 4l-7 7M4 20l7-7" />
          </svg>
        </div>
      </div>
      {!hasTrack && (
        <div className="mini-hint" data-tauri-drag-region>
          Nothing playing
        </div>
      )}
    </div>
  );
}
