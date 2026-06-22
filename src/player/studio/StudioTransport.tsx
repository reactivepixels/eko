import { useEffect, useRef } from "react";
import { useUiStore } from "../../store/useUiStore";
import { formatTime } from "../../lib/format";
import { LocalCover } from "../LocalCover";
import { Marquee } from "../Marquee";
import { useNowPlaying } from "../../hooks/useNowPlaying";
import { useTransport } from "../../hooks/useTransport";
import { useVolume } from "../../hooks/useVolume";
import { useScrub } from "../../hooks/useScrub";
import { useSpectrum } from "../../hooks/useSpectrum";
import styles from "./StudioTransport.module.css";

/**
 * Green LED level meter — ~12 vertical bars, off/on driven by live FFT bands
 * collapsed to a single level. Uses a requestAnimationFrame loop with exponential
 * smoothing. Bars 11–12 (top 2) are marked as peak and glow accent when lit.
 */
const METER_SEGS = 12;

function DockMeter() {
  const { read } = useSpectrum();
  const barsRef = useRef<(HTMLSpanElement | null)[]>([]);

  useEffect(() => {
    let raf = 0;
    let smoothed = 0;

    const tick = () => {
      const bands = read();
      let target: number;
      if (bands && bands.length > 0) {
        // collapse all bands to a single level via RMS-ish mean
        let sum = 0;
        for (let i = 0; i < bands.length; i++) sum += bands[i];
        target = sum / bands.length;
      } else {
        target = 0;
      }
      // exponential smoothing: fast attack, slow release
      if (target > smoothed) {
        smoothed = smoothed * 0.5 + target * 0.5;
      } else {
        smoothed = smoothed * 0.88 + target * 0.12;
      }
      // clamp
      if (smoothed < 0) smoothed = 0;
      if (smoothed > 1) smoothed = 1;

      const lit = Math.round(smoothed * METER_SEGS);
      for (let i = 0; i < METER_SEGS; i++) {
        const el = barsRef.current[i];
        if (!el) continue;
        const isLit = i < lit;
        el.dataset.lit = isLit ? "1" : "0";
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [read]);

  return (
    <div className={styles.dkMeter}>
      {Array.from({ length: METER_SEGS }, (_, i) => (
        <span
          key={i}
          className={`${styles.mBar}${i >= METER_SEGS - 2 ? ` ${styles.mBarPk}` : ""}`}
          ref={(el) => {
            barsRef.current[i] = el;
          }}
          data-lit="0"
        />
      ))}
    </div>
  );
}

/**
 * Studio transport — same hooks and arrangement as Porcelain's TransportBar (left = now-playing,
 * center = controls + seek, right = LED meter + VOL label + volume knob + queue) but rendered with
 * the Studio dock material: flat-top puck buttons, a recessed seek slot, a multi-layer rotary
 * volume knob, and a segmented LED meter. All classes are locally scoped via CSS Modules.
 */
export function StudioTransport() {
  const np = useNowPlaying();
  const tr = useTransport();
  const vol = useVolume();
  const scrub = useScrub();
  const toggleQueue = useUiStore((s) => s.toggleQueue);

  const prog = scrub.progress;

  return (
    <footer className={styles.bar}>
      {/* LEFT — now-playing info */}
      <div className={styles.np} onClick={np.openDeck} title="Now Playing">
        <div className={styles.art}>
          {np.coverUrl(120) ? (
            <img src={np.coverUrl(120) ?? ""} alt="" />
          ) : np.coverPath ? (
            <LocalCover path={np.coverPath} />
          ) : null}
        </div>
        <div className={styles.npText}>
          <div className={styles.tt}>
            <Marquee text={np.title} />
          </div>
          <div className={styles.ar}>
            {np.hasTrack ? (np.artist ?? "") : "Pick an album to play"}
          </div>
        </div>
      </div>

      {/* CENTER — controls row + seek bar underneath */}
      <div className={styles.center}>
        <div className={`${styles.controls}${!tr.hasQueue ? ` ${styles.dim}` : ""}`}>
          {/* Shuffle */}
          <button
            className={`${styles.puck} ${styles.puckSm}${tr.shuffle ? ` ${styles.on}` : ""}`}
            title="Shuffle"
            onClick={tr.toggleShuffle}
          >
            <span className={styles.puckTop}>
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M3 7h4l9 10h5M16 7h5v5M3 17h4l3-3.4" />
              </svg>
            </span>
          </button>

          {/* Previous */}
          <button className={styles.puck} title="Previous" onClick={tr.prev}>
            <span className={styles.puckTop}>
              <svg viewBox="0 0 24 24" fill="currentColor">
                <path d="M6 5v14h2V5H6zm3 7 10 7V5L9 12z" />
              </svg>
            </span>
          </button>

          {/* Play / Pause */}
          <button
            className={`${styles.puck} ${styles.puckPlay}`}
            title="Play / Pause"
            onClick={tr.togglePlay}
          >
            <span className={styles.puckTop}>
              {tr.isPlaying ? (
                <svg viewBox="0 0 24 24" fill="currentColor">
                  <rect x="6" y="5" width="4" height="14" rx="1" />
                  <rect x="14" y="5" width="4" height="14" rx="1" />
                </svg>
              ) : (
                <svg viewBox="0 0 24 24" fill="currentColor">
                  <path d="M8 5v14l11-7z" />
                </svg>
              )}
            </span>
          </button>

          {/* Next */}
          <button className={styles.puck} title="Next" onClick={tr.next}>
            <span className={styles.puckTop}>
              <svg viewBox="0 0 24 24" fill="currentColor">
                <path d="M18 5v14h-2V5h2zM15 12 5 5v14l10-7z" />
              </svg>
            </span>
          </button>

          {/* Repeat */}
          <button
            className={`${styles.puck} ${styles.puckSm}${tr.repeat !== "off" ? ` ${styles.on}` : ""}`}
            title="Repeat"
            onClick={tr.cycleRepeat}
          >
            <span className={styles.puckTop}>
              <svg
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              >
                <path d="M4 9a4 4 0 0 1 4-4h9M17 5l-2-2M17 5l-2 2M20 15a4 4 0 0 1-4 4H7M7 19l2-2M7 19l2 2" />
              </svg>
            </span>
          </button>
        </div>

        {/* Seek / scrub bar — recessed slot, directly under controls */}
        <div className={styles.seekwrap}>
          <span className={styles.time}>{formatTime(scrub.currentTime)}</span>
          <div
            className={styles.slot}
            onPointerDown={scrub.onPointerDown}
            onPointerMove={scrub.onPointerMove}
            onPointerUp={scrub.onPointerUp}
            onPointerCancel={scrub.onPointerCancel}
          >
            <div className={styles.slotFill} style={{ width: `${prog * 100}%` }} />
            <div className={styles.slotNub} style={{ left: `${prog * 100}%` }} />
          </div>
          <span className={`${styles.time} ${styles.timeR}`}>-{formatTime(scrub.remaining)}</span>
        </div>
      </div>

      {/* RIGHT — same slot order as Porcelain for cross-theme structural consistency:
          queue → mini-eq (meter) → volume */}
      <div className={styles.right}>
        {/* Queue */}
        <button className={styles.ibtn} title="Up Next" onClick={toggleQueue}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M4 7h11M4 12h11M4 17h7" />
            <path d="M16 13.5v7l5-3.5z" fill="currentColor" stroke="none" />
          </svg>
        </button>

        {/* Green LED segment level meter */}
        <DockMeter />

        {/* VOL label + volume knob */}
        <div className={styles.dkVol}>
          <span className={styles.vlab}>Vol</span>

          {/* Volume knob — multi-layer: gauge SVG + convex body + flat cap + dial/indicator */}
          <div
            className={styles.vol}
            title="Volume (drag or scroll)"
            onPointerDown={vol.onPointerDown}
            onWheel={vol.onWheel}
          >
            <svg className={styles.volArc} viewBox="0 0 100 100">
              <circle className={styles.volTrk} cx="50" cy="50" r="45" pathLength="100" />
              <circle
                className={styles.volVal}
                cx="50"
                cy="50"
                r="45"
                pathLength="100"
                style={{
                  strokeDasharray: `${(((vol.deg + 135) / 270) * 75).toFixed(1)} 100`,
                }}
              />
            </svg>
            <div className={styles.volBody} />
            <div className={styles.volCap} />
            <div className={styles.volDial} style={{ transform: `rotate(${vol.deg}deg)` }}>
              <span className={styles.volInd} />
            </div>
          </div>
        </div>
      </div>
    </footer>
  );
}
