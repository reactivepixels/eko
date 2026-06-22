import { useState } from "react";
import { LocalCover } from "../LocalCover";
import { useContextMenu } from "../ContextMenu";
import { useQueue } from "../../hooks/useQueue";
import styles from "./StudioQueue.module.css";

/**
 * Studio skin "Up Next" queue drawer — same hooks and behavior as QueuePanel
 * (drag-to-reorder, remove, clear, click-to-play, context menu) rendered with
 * the Studio matte slide-over drawer material from the approved concept.
 */
export function StudioQueue() {
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
      <div className={styles.scrim} onClick={q.close} />
      <aside className={styles.drawer}>
        <div className={styles.head}>
          <b className={styles.headTitle}>Up Next</b>
          <span className={styles.headCount}>
            {q.tracks.length} {q.tracks.length === 1 ? "track" : "tracks"}
          </span>
          {q.tracks.length > 0 && (
            <button className={styles.clearBtn} onClick={q.clear} title="Clear queue">
              Clear
            </button>
          )}
          <button className={styles.closeBtn} onClick={q.close} title="Close">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M6 6l12 12M18 6L6 18" />
            </svg>
          </button>
        </div>

        <div className={styles.list}>
          {q.tracks.length === 0 ? (
            <div className={styles.empty}>Nothing queued.</div>
          ) : (
            q.tracks.map((t, i) => {
              const cover = q.coverUrl(t, 80);
              const isPlaying = i === q.currentIndex;
              return (
                <div
                  key={t.id}
                  className={[
                    styles.qrow,
                    isPlaying ? styles.playing : "",
                    over === i ? styles.dropOver : "",
                    drag === i ? styles.dragging : "",
                  ]
                    .filter(Boolean)
                    .join(" ")}
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
                  <div className={styles.thumb}>
                    {cover ? (
                      <img src={cover} alt="" />
                    ) : t.path && !t.subsonicId ? (
                      <LocalCover path={t.path} />
                    ) : null}
                  </div>
                  <div className={styles.meta}>
                    <div className={styles.title}>{t.title ?? "—"}</div>
                    <div className={styles.artist}>{t.artist ?? ""}</div>
                  </div>
                  <div
                    className={styles.removeBtn}
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
