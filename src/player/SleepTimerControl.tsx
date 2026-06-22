import { useState } from "react";
import { usePlayerStore, SLEEP_PRESETS, type SleepPreset } from "../store/usePlayerStore";
import { formatTime } from "../lib/format";

/** Pill showing remaining sleep-timer time and a cancel button. */
function SleepTimerPill({ onCancel }: { onCancel: () => void }) {
  const sleepTimer = usePlayerStore((s) => s.sleepTimer);
  if (!sleepTimer) return null;

  const label = sleepTimer.endOfTrack
    ? "End of track"
    : sleepTimer.remainingSec != null
      ? formatTime(sleepTimer.remainingSec)
      : "";

  return (
    <div className="sleep-pill" role="status" aria-label={`Sleep timer: ${label}`}>
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        aria-hidden="true"
        className="sleep-pill__icon"
      >
        <path d="M12 2a10 10 0 1 0 10 10" />
        <path d="M12 6v6l4 2" />
        <path d="M18 2l2 2-2 2M20 2l2 2-2 2" />
      </svg>
      <span className="sleep-pill__label">{label}</span>
      <div
        className="sleep-pill__cancel"
        onClick={(e) => {
          e.stopPropagation();
          onCancel();
        }}
        role="button"
        tabIndex={0}
        aria-label="Cancel sleep timer"
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.stopPropagation();
            onCancel();
          }
        }}
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          aria-hidden="true"
        >
          <path d="M6 6l12 12M18 6L6 18" />
        </svg>
      </div>
    </div>
  );
}

/** Sleep-timer button + dropdown. Lives in the transport bar right-side controls. */
export function SleepTimerControl() {
  const sleepTimer = usePlayerStore((s) => s.sleepTimer);
  const startSleepTimer = usePlayerStore((s) => s.startSleepTimer);
  const cancelSleepTimer = usePlayerStore((s) => s.cancelSleepTimer);
  const [open, setOpen] = useState(false);

  const active = sleepTimer !== null;

  if (active && !open) {
    // When the timer is active, show the pill inline and allow cancellation.
    return <SleepTimerPill onCancel={cancelSleepTimer} />;
  }

  const pick = (preset: SleepPreset) => {
    startSleepTimer(preset);
    setOpen(false);
  };

  return (
    <div className="sleep-ctl" style={{ position: "relative" }}>
      <div
        className={`icon-btn${active ? " on" : ""}`}
        title="Sleep timer"
        onClick={() => setOpen((v) => !v)}
        role="button"
        tabIndex={0}
        aria-label="Sleep timer"
        aria-expanded={open}
        aria-haspopup="listbox"
        onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? setOpen((v) => !v) : undefined)}
      >
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          aria-hidden="true"
        >
          <circle cx="12" cy="12" r="9" />
          <path d="M12 7v5l3 3" />
        </svg>
      </div>

      {open && (
        <>
          <div className="backdrop" onClick={() => setOpen(false)} />
          <div className="menu sleep-menu" role="listbox" aria-label="Sleep timer presets">
            <div className="sleep-menu__head">SLEEP TIMER</div>
            {SLEEP_PRESETS.map((mins) => (
              <div
                key={mins}
                className="mi"
                role="option"
                tabIndex={0}
                aria-selected={false}
                onClick={() => pick(mins)}
                onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? pick(mins) : undefined)}
              >
                {mins} min
              </div>
            ))}
            <div
              className="mi"
              role="option"
              tabIndex={0}
              aria-selected={false}
              onClick={() => pick(-1)}
              onKeyDown={(e) => (e.key === "Enter" || e.key === " " ? pick(-1) : undefined)}
            >
              End of track
            </div>
            {active && (
              <div
                className="mi sleep-menu__cancel"
                role="option"
                tabIndex={0}
                aria-selected={false}
                onClick={() => {
                  cancelSleepTimer();
                  setOpen(false);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    cancelSleepTimer();
                    setOpen(false);
                  }
                }}
              >
                Cancel timer
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
