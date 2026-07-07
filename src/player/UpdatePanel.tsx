import { useUpdaterStore } from "../store/useUpdaterStore";

// ---------------------------------------------------------------------------
// UpdatePanel — "Check for Updates" section in the sidebar
//
// Rendered below ProPanel. The panel is always visible so users can manually
// trigger a check. On launch a silent background check runs from App.tsx.
//
// States:
//   idle        → "Check for Updates" button only
//   checking    → spinner, disabled button
//   available   → version + notes + "Download & Install" CTA
//   downloading → progress bar, no user action
//   ready       → "Relaunching…" (relaunch fires automatically after install)
//   error       → error message + retry button
// ---------------------------------------------------------------------------

export function UpdatePanel() {
  const {
    phase,
    availableVersion,
    releaseNotes,
    progress,
    error,
    checkManual,
    installAndRelaunch,
    dismiss,
  } = useUpdaterStore();

  // "up to date" banner shown briefly when a manual check finds no update
  const isUpToDate = phase === "idle";
  const isChecking = phase === "checking";
  const isAvailable = phase === "available";
  const isDownloading = phase === "downloading";
  const isReady = phase === "ready";
  const isError = phase === "error";

  return (
    <div className="dac-card update-panel">
      <div className="t">UPDATES</div>

      {/* ── Idle: just the check button ─────────────────── */}
      {isUpToDate && (
        <div className="update-actions">
          <span
            className="pro-action-link"
            onClick={() => void checkManual()}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") void checkManual();
            }}
          >
            Check for Updates
          </span>
        </div>
      )}

      {/* ── Checking ────────────────────────────────────── */}
      {isChecking && (
        <div className="update-status">
          <span className="update-spinner" aria-label="Checking…" />
          <span className="update-status__label">Checking…</span>
        </div>
      )}

      {/* ── Update available ────────────────────────────── */}
      {isAvailable && (
        <div className="update-available">
          <div className="update-version">v{availableVersion} available</div>
          {releaseNotes && <div className="update-notes">{releaseNotes}</div>}
          <button className="pillbtn update-install-btn" onClick={() => void installAndRelaunch()}>
            <svg
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M12 5v14M5 12l7 7 7-7" />
            </svg>
            Download &amp; Install
          </button>
          <span
            className="update-later"
            onClick={dismiss}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") dismiss();
            }}
          >
            Later
          </span>
        </div>
      )}

      {/* ── Downloading ─────────────────────────────────── */}
      {isDownloading && (
        <div className="update-status">
          <div className="update-progress-bar">
            <div className="update-progress-bar__fill" style={{ width: `${progress ?? 0}%` }} />
          </div>
          <span className="update-status__label">
            {progress !== null ? `${progress}%` : "Downloading…"}
          </span>
        </div>
      )}

      {/* ── Ready / relaunching ─────────────────────────── */}
      {isReady && (
        <div className="update-status">
          <span className="update-status__label">Relaunching…</span>
        </div>
      )}

      {/* ── Error ───────────────────────────────────────── */}
      {isError && (
        <div className="update-error">
          <div className="pro-error">{error}</div>
          <div className="update-actions">
            <span
              className="pro-action-link"
              onClick={() => void checkManual()}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") void checkManual();
              }}
            >
              Retry
            </span>
            <span
              className="pro-action-link"
              onClick={dismiss}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") dismiss();
              }}
            >
              Dismiss
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
