import { usePlayerStore } from "../store/usePlayerStore";
import { formatTime } from "../lib/format";

/**
 * Sleep-timer indicator — a compact pill in the transport that appears ONLY while a
 * timer is armed, showing the remaining time (or "End of track") with a cancel button.
 *
 * The picker moved to the native "Controls ▸ Sleep Timer" menu (see useSleepTimerMenu),
 * so the transport no longer carries an always-on clock button taking up prime space —
 * this renders nothing until a timer is actually running.
 */
export function SleepTimerControl() {
  const sleepTimer = usePlayerStore((s) => s.sleepTimer);
  const cancelSleepTimer = usePlayerStore((s) => s.cancelSleepTimer);

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
          cancelSleepTimer();
        }}
        role="button"
        tabIndex={0}
        aria-label="Cancel sleep timer"
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.stopPropagation();
            cancelSleepTimer();
          }
        }}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
          <path d="M6 6l12 12M18 6L6 18" />
        </svg>
      </div>
    </div>
  );
}
