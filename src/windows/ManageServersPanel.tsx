import { useState, type CSSProperties } from "react";
import { useSubsonic } from "../subsonic/useSubsonic";
import { ConnectPanel } from "./ConnectPanel";

/** Manage-servers overlay: list all servers, switch active, rename, remove, add new. */
export function ManageServersPanel() {
  const serverList = useSubsonic((s) => s.serverList);
  const manageOpen = useSubsonic((s) => s.manageOpen);
  const setManageOpen = useSubsonic((s) => s.setManageOpen);
  const switchServer = useSubsonic((s) => s.switchServer);
  const removeServerFn = useSubsonic((s) => s.removeServer);
  const renameServerFn = useSubsonic((s) => s.renameServer);
  const status = useSubsonic((s) => s.status);

  const [addingNew, setAddingNew] = useState(false);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [switchingId, setSwitchingId] = useState<string | null>(null);

  if (!manageOpen) return null;

  const close = () => {
    setManageOpen(false);
    setAddingNew(false);
    setRenamingId(null);
  };

  if (addingNew) {
    return (
      <ConnectPanel
        mode="add"
        onSuccess={() => {
          setAddingNew(false);
          setManageOpen(false);
        }}
        onCancel={() => setAddingNew(false)}
      />
    );
  }

  const startRename = (id: string, currentName: string) => {
    setRenamingId(id);
    setRenameValue(currentName);
  };

  const commitRename = (id: string) => {
    if (renameValue.trim()) renameServerFn(id, renameValue.trim());
    setRenamingId(null);
  };

  const handleSwitch = async (id: string) => {
    setSwitchingId(id);
    await switchServer(id);
    setSwitchingId(null);
    setManageOpen(false);
  };

  const handleRemove = async (id: string) => {
    await removeServerFn(id);
  };

  return (
    <div style={overlay} onClick={close}>
      <div style={card} onClick={(e) => e.stopPropagation()}>
        {/* Header */}
        <div style={headRow}>
          <div style={brand}>EKO</div>
          <div style={closeBtn} onClick={close} title="Close">
            ✕
          </div>
        </div>
        <div style={subTitle}>SERVERS</div>

        {/* Server list */}
        <div style={listWrap}>
          {serverList.servers.length === 0 && <div style={emptyMsg}>No servers configured.</div>}
          {serverList.servers.map((srv) => {
            const isActive = srv.id === serverList.activeId;
            const isSwitching = switchingId === srv.id && status === "connecting";
            const isRenaming = renamingId === srv.id;

            return (
              <div key={srv.id} style={{ ...row, ...(isActive ? rowActive : {}) }}>
                {/* Active indicator */}
                <div style={ledWrap}>{isActive && <span style={led} aria-hidden="true" />}</div>

                {/* Name / rename field */}
                <div style={rowMain}>
                  {isRenaming ? (
                    <input
                      style={renameInput}
                      value={renameValue}
                      autoFocus
                      onChange={(e) => setRenameValue(e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") commitRename(srv.id);
                        if (e.key === "Escape") setRenamingId(null);
                      }}
                      onBlur={() => commitRename(srv.id)}
                    />
                  ) : (
                    <div style={srvName}>{srv.name}</div>
                  )}
                  <div style={srvMeta}>
                    {srv.username} · {srv.baseUrl}
                  </div>
                </div>

                {/* Actions */}
                <div style={rowActions}>
                  {!isActive && (
                    <button
                      style={actionBtn}
                      onClick={() => void handleSwitch(srv.id)}
                      disabled={isSwitching || status === "connecting"}
                      title="Switch to this server"
                    >
                      {isSwitching ? "…" : "USE"}
                    </button>
                  )}
                  <button
                    style={actionBtn}
                    onClick={() => startRename(srv.id, srv.name)}
                    title="Rename"
                  >
                    RENAME
                  </button>
                  <button
                    style={{ ...actionBtn, ...actionBtnDanger }}
                    onClick={() => void handleRemove(srv.id)}
                    title="Remove server"
                  >
                    REMOVE
                  </button>
                </div>
              </div>
            );
          })}
        </div>

        {/* Add server */}
        <button style={addBtn} onClick={() => setAddingNew(true)}>
          + ADD SERVER
        </button>
      </div>
    </div>
  );
}

// ── Styles ────────────────────────────────────────────────────────────────────

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
  width: 480,
  maxHeight: "80vh",
  background: "#eceae1",
  borderRadius: 10,
  padding: "22px 24px",
  boxShadow: "0 20px 50px rgba(0,0,0,.5)",
  fontFamily: "'Helvetica Neue',Helvetica,Arial,sans-serif",
  display: "flex",
  flexDirection: "column",
  gap: 14,
  border: "1px solid #c8c6bd",
  overflow: "hidden",
};

const headRow: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
};

const brand: CSSProperties = {
  fontSize: 18,
  fontWeight: 600,
  letterSpacing: 3,
  color: "#23231f",
};

const closeBtn: CSSProperties = {
  cursor: "pointer",
  color: "#9b9a92",
  fontSize: 13,
  padding: "2px 6px",
  borderRadius: 6,
};

const subTitle: CSSProperties = {
  fontSize: 9,
  letterSpacing: 2,
  color: "#86867e",
  marginTop: -6,
};

const listWrap: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 6,
  overflowY: "auto",
  maxHeight: "50vh",
};

const emptyMsg: CSSProperties = {
  fontSize: 12,
  color: "#9b9a92",
  padding: "8px 0",
};

const row: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 10,
  padding: "10px 12px",
  borderRadius: 8,
  background: "#f2f0e8",
  border: "1px solid #d8d6cd",
};

const rowActive: CSSProperties = {
  border: "1px solid #ef6a1e",
  background: "#fff8f3",
};

const ledWrap: CSSProperties = {
  width: 10,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  flexShrink: 0,
};

const led: CSSProperties = {
  width: 6,
  height: 6,
  borderRadius: "50%",
  background: "#ef6a1e",
  boxShadow: "0 0 5px #ef6a1e",
};

const rowMain: CSSProperties = {
  flex: 1,
  minWidth: 0,
  display: "flex",
  flexDirection: "column",
  gap: 2,
};

const srvName: CSSProperties = {
  fontSize: 13,
  fontWeight: 600,
  color: "#23231f",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
};

const srvMeta: CSSProperties = {
  fontSize: 10,
  color: "#9b9a92",
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
  fontFamily: "ui-monospace,monospace",
};

const renameInput: CSSProperties = {
  fontSize: 13,
  fontWeight: 600,
  padding: "2px 6px",
  borderRadius: 4,
  border: "1px solid #c0beb5",
  background: "#fbfaf4",
  color: "#23231f",
  fontFamily: "inherit",
  width: "100%",
};

const rowActions: CSSProperties = {
  display: "flex",
  gap: 6,
  flexShrink: 0,
};

const actionBtn: CSSProperties = {
  fontSize: 9,
  letterSpacing: 1,
  fontWeight: 700,
  padding: "5px 9px",
  borderRadius: 4,
  border: "1px solid #c0beb5",
  background: "#eceae1",
  color: "#686762",
  cursor: "pointer",
  fontFamily: "inherit",
};

const actionBtnDanger: CSSProperties = {
  color: "#b3402f",
  borderColor: "#e8b0aa",
  background: "#fdf3f2",
};

const addBtn: CSSProperties = {
  padding: "9px",
  borderRadius: 4,
  border: "1px dashed #c0beb5",
  cursor: "pointer",
  background: "transparent",
  color: "#ef6a1e",
  fontWeight: 700,
  letterSpacing: 1.5,
  fontSize: 11,
  fontFamily: "inherit",
};
