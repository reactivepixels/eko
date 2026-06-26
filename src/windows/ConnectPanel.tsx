import { useState, type FormEvent } from "react";
import { useSubsonic } from "../subsonic/useSubsonic";
import { useUiStore } from "../store/useUiStore";
import type { SubsonicConfig } from "../subsonic/client";

interface ConnectPanelProps {
  /**
   * "initial" (default) — first-time / re-connect flow; uses addAndConnect so the
   *   server is added to the list.
   * "add" — explicitly adding a new server from the manage panel; same underlying
   *   call, the title text differs.
   */
  mode?: "initial" | "add";
  /** Called after a successful connection in "add" mode. */
  onSuccess?: () => void;
  /** Called when the user dismisses the panel in "add" mode (no fallback-to-local). */
  onCancel?: () => void;
}

/** Connection form shown when EKO isn't connected to a Navidrome/OpenSubsonic server,
 *  or when the user is adding a new server from the manage-servers panel. */
export function ConnectPanel({ mode = "initial", onSuccess, onCancel }: ConnectPanelProps) {
  const addAndConnect = useSubsonic((s) => s.addAndConnect);
  const status = useSubsonic((s) => s.status);
  const error = useSubsonic((s) => s.error);
  // The panel renders outside .app, so carry the active theme context onto the
  // overlay itself — that's what makes its tokens follow skin + accent + light/dark.
  const theme = useUiStore((s) => s.theme);
  const accent = useUiStore((s) => s.accent);
  const skin = useUiStore((s) => s.skin);
  const [baseUrl, setBaseUrl] = useState("http://192.168.86.50:4533");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [serverName, setServerName] = useState("");

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const cfg: SubsonicConfig = { baseUrl: baseUrl.trim(), username: username.trim(), password };
    const ok = await addAndConnect(serverName.trim() || undefined, cfg);
    if (ok && onSuccess) onSuccess();
  };

  const dismiss = () => {
    if (onCancel) {
      onCancel();
    } else {
      // Default initial behaviour: fall back to local source.
      useUiStore.getState().setSource("local");
    }
  };

  const title = mode === "add" ? "ADD SERVER" : "CONNECT TO NAVIDROME";

  return (
    <div
      className="connect-overlay"
      data-theme={theme}
      data-accent={accent}
      data-skin={skin}
      onClick={dismiss}
    >
      <form className="connect-card" onSubmit={submit} onClick={(e) => e.stopPropagation()}>
        <div className="connect-head">
          <div className="connect-brand">EKO</div>
          <div
            className="connect-close"
            onClick={dismiss}
            title={mode === "add" ? "Cancel" : "Back to Local"}
          >
            ✕
          </div>
        </div>
        <div className="connect-sub">{title}</div>
        <label className="connect-field">
          SERVER NAME (optional)
          <input
            value={serverName}
            onChange={(e) => setServerName(e.target.value)}
            placeholder="My Navidrome"
          />
        </label>
        <label className="connect-field">
          SERVER URL
          <input
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="http://host:4533"
            autoFocus
          />
        </label>
        <label className="connect-field">
          USERNAME
          <input value={username} onChange={(e) => setUsername(e.target.value)} />
        </label>
        <label className="connect-field">
          PASSWORD
          <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
        </label>
        <button type="submit" className="connect-btn" disabled={status === "connecting"}>
          {status === "connecting" ? "CONNECTING…" : mode === "add" ? "ADD SERVER" : "CONNECT"}
        </button>
        {error && <div className="connect-err">✕ {error}</div>}
        {mode === "initial" && (
          <div className="connect-back" onClick={dismiss}>
            ← Use local files instead
          </div>
        )}
      </form>
    </div>
  );
}
