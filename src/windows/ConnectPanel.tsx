import { useState, type CSSProperties, type FormEvent } from "react";
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
    <div style={overlay} onClick={dismiss}>
      <form onSubmit={submit} style={card} onClick={(e) => e.stopPropagation()}>
        <div style={headRow}>
          <div style={brand}>EKO</div>
          <div
            style={closeBtn}
            onClick={dismiss}
            title={mode === "add" ? "Cancel" : "Back to Local"}
          >
            ✕
          </div>
        </div>
        <div style={sub}>{title}</div>
        <label style={lbl}>
          SERVER NAME (optional)
          <input
            style={input}
            value={serverName}
            onChange={(e) => setServerName(e.target.value)}
            placeholder="My Navidrome"
          />
        </label>
        <label style={lbl}>
          SERVER URL
          <input
            style={input}
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="http://host:4533"
            autoFocus
          />
        </label>
        <label style={lbl}>
          USERNAME
          <input style={input} value={username} onChange={(e) => setUsername(e.target.value)} />
        </label>
        <label style={lbl}>
          PASSWORD
          <input
            style={input}
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
        <button type="submit" style={btn} disabled={status === "connecting"}>
          {status === "connecting" ? "CONNECTING…" : mode === "add" ? "ADD SERVER" : "CONNECT"}
        </button>
        {error && <div style={errStyle}>✕ {error}</div>}
        {mode === "initial" && (
          <div style={backLink} onClick={dismiss}>
            ← Use local files instead
          </div>
        )}
      </form>
    </div>
  );
}

const headRow: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
};
const closeBtn: CSSProperties = {
  cursor: "pointer",
  color: "#9b9a92",
  fontSize: 13,
  padding: "2px 6px",
  borderRadius: 6,
};
const backLink: CSSProperties = {
  marginTop: 4,
  fontSize: 11,
  color: "#86867e",
  cursor: "pointer",
  textAlign: "center",
};

const overlay: CSSProperties = {
  position: "fixed",
  inset: 0,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  background: "rgba(20,20,18,.55)",
  zIndex: 100,
};
const card: CSSProperties = {
  width: 300,
  background: "#eceae1",
  borderRadius: 10,
  padding: "22px 24px",
  boxShadow: "0 20px 50px rgba(0,0,0,.5)",
  fontFamily: "'Helvetica Neue',Helvetica,Arial,sans-serif",
  display: "flex",
  flexDirection: "column",
  gap: 12,
  border: "1px solid #c8c6bd",
};
const brand: CSSProperties = { fontSize: 18, fontWeight: 600, letterSpacing: 3, color: "#23231f" };
const sub: CSSProperties = {
  fontSize: 9,
  letterSpacing: 2,
  color: "#86867e",
  marginTop: -6,
  marginBottom: 4,
};
const lbl: CSSProperties = {
  fontSize: 8,
  letterSpacing: 1.5,
  color: "#7a7a72",
  display: "flex",
  flexDirection: "column",
  gap: 4,
};
const input: CSSProperties = {
  fontSize: 13,
  padding: "7px 9px",
  borderRadius: 4,
  border: "1px solid #c0beb5",
  background: "#fbfaf4",
  color: "#23231f",
  fontFamily: "inherit",
};
const btn: CSSProperties = {
  marginTop: 6,
  padding: "9px",
  borderRadius: 4,
  border: "none",
  cursor: "pointer",
  background: "#ef6a1e",
  color: "#fff",
  fontWeight: 600,
  letterSpacing: 1.5,
  fontSize: 11,
};
const errStyle: CSSProperties = {
  fontSize: 11,
  color: "#b3402f",
  fontFamily: "ui-monospace,monospace",
};
