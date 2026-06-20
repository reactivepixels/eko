import { useState } from "react";
import { LocalCover } from "./LocalCover";
import { useContextMenu } from "./ContextMenu";
import { useQueue } from "../hooks/useQueue";

/** Porcelain "Up Next" panel — pure presentation over `useQueue()`. Owns only the drag-state. */
export function QueuePanel() {
  const q = useQueue();
  const { open: openMenu, menu } = useContextMenu();

  const [drag, setDrag] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);

  if (!q.isOpen) return null;

  const drop = (to: number) => {
    if (drag != null && drag !== to) q.reorder(drag, to);
    setDrag(null);
    setOver(null);
  };

  return (
    <>
      <div className="q-backdrop" onClick={q.close} />
      <aside className="queue">
        <div className="queue-head">
          <b>Up Next</b>
          <span className="q-count">
            {q.tracks.length} {q.tracks.length === 1 ? "track" : "tracks"}
          </span>
          {q.tracks.length > 0 && (
            <div className="q-clear" onClick={q.clear} title="Clear queue">
              Clear
            </div>
          )}
          <div className="q-close icon-btn" onClick={q.close} title="Close">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </div>
        </div>
        <div className="queue-list">
          {q.tracks.length === 0 ? (
            <div className="empty">Nothing queued.</div>
          ) : (
            q.tracks.map((t, i) => {
              const cover = q.coverUrl(t, 80);
              return (
                <div
                  key={t.id}
                  className={`qrow${i === q.currentIndex ? " playing" : ""}${over === i ? " drop-over" : ""}${drag === i ? " dragging" : ""}`}
                  draggable
                  onDragStart={() => setDrag(i)}
                  onDragOver={(e) => {
                    e.preventDefault();
                    if (over !== i) setOver(i);
                  }}
                  onDragEnd={() => {
                    setDrag(null);
                    setOver(null);
                  }}
                  onDrop={(e) => {
                    e.preventDefault();
                    drop(i);
                  }}
                  onClick={() => q.playAt(i)}
                  onContextMenu={openMenu(q.rowMenuItems(t.id, i))}
                >
                  <div className="qart">
                    {cover ? (
                      <img src={cover} alt="" />
                    ) : t.path && !t.subsonicId ? (
                      <LocalCover path={t.path} />
                    ) : null}
                  </div>
                  <div className="qmeta">
                    <div className="qtt">{t.title ?? "—"}</div>
                    <div className="qar">{t.artist ?? ""}</div>
                  </div>
                  <div
                    className="qrm"
                    onClick={(e) => {
                      e.stopPropagation();
                      q.remove(t.id);
                    }}
                    title="Remove"
                  >
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                      <path d="M6 6l12 12M18 6L6 18" />
                    </svg>
                  </div>
                </div>
              );
            })
          )}
        </div>
        {menu}
      </aside>
    </>
  );
}
