import { useState } from "react";
import { usePlayerStore } from "../store/usePlayerStore";
import { useUiStore } from "../store/useUiStore";
import { coverArtUrl } from "../subsonic/client";
import { LocalCover } from "./LocalCover";
import { useContextMenu } from "./ContextMenu";

export function QueuePanel() {
  const open = useUiStore((s) => s.queueOpen);
  const close = useUiStore((s) => s.toggleQueue);
  const tracks = usePlayerStore((s) => s.tracks);
  const curIdx = usePlayerStore((s) => s.currentIndex);
  const { open: openMenu, menu } = useContextMenu();

  const [drag, setDrag] = useState<number | null>(null);
  const [over, setOver] = useState<number | null>(null);

  if (!open) return null;

  const drop = (to: number) => {
    if (drag != null && drag !== to) usePlayerStore.getState().reorder(drag, to);
    setDrag(null);
    setOver(null);
  };

  // Right-click a queued row → play it now or drop it from the queue.
  const rowMenu = (id: string, i: number) =>
    openMenu([
      { label: "Play now", onSelect: () => void usePlayerStore.getState().playAt(i) },
      { separator: true },
      {
        label: "Remove from queue",
        danger: true,
        onSelect: () => usePlayerStore.getState().removeTrack(id),
      },
    ]);

  return (
    <>
      <div className="q-backdrop" onClick={close} />
      <aside className="queue">
        <div className="queue-head">
          <b>Up Next</b>
          <span className="q-count">
            {tracks.length} {tracks.length === 1 ? "track" : "tracks"}
          </span>
          {tracks.length > 0 && (
            <div
              className="q-clear"
              onClick={() => usePlayerStore.getState().clearPlaylist()}
              title="Clear queue"
            >
              Clear
            </div>
          )}
          <div className="q-close icon-btn" onClick={close} title="Close">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </div>
        </div>
        <div className="queue-list">
          {tracks.length === 0 ? (
            <div className="empty">Nothing queued.</div>
          ) : (
            tracks.map((t, i) => {
              const cover = t.coverArt ? coverArtUrl(t.coverArt, 80) : null;
              return (
                <div
                  key={t.id}
                  className={`qrow${i === curIdx ? " playing" : ""}${over === i ? " drop-over" : ""}${drag === i ? " dragging" : ""}`}
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
                  onClick={() => void usePlayerStore.getState().playAt(i)}
                  onContextMenu={rowMenu(t.id, i)}
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
                      usePlayerStore.getState().removeTrack(t.id);
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
