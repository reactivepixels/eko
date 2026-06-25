/**
 * PorcelainShell — the default (free) theme's Shell.
 *
 * Owns the full Porcelain main-window composition: top bar + sidebar + main (Library /
 * Now Playing) + transport + queue + lyrics. Reads the shared headless hooks; this is the
 * body that previously lived inline in `PlayerApp` as the children of `StudioApp`.
 *
 * Registered via `porcelainTheme` (see src/skin/registry.ts + src/skin/registerThemes.ts).
 */
import { useUiStore } from "../../store/useUiStore";
import { useMusicSource } from "../../hooks/useMusicSource";
import { TopBar } from "../TopBar";
import { Sidebar } from "../Sidebar";
import { LibraryView } from "../LibraryView";
import { DeckShell } from "../DeckShell";
import { TransportShell } from "../TransportShell";
import { QueuePanel } from "../QueuePanel";
import { LyricsPanel } from "../LyricsPanel";

export function PorcelainShell() {
  const playerView = useUiStore((s) => s.playerView);
  const setPlayerView = useUiStore((s) => s.setPlayerView);
  const libSection = useUiStore((s) => s.libSection);
  const lyricsOpen = useUiStore((s) => s.lyricsOpen);
  const toggleLyrics = useUiStore((s) => s.toggleLyrics);
  const count = useMusicSource().albumCount;

  return (
    <>
      <TopBar />
      <Sidebar />
      <main className="main">
        <div className="main-head">
          <div className="viewseg" role="group" aria-label="Main view">
            <b
              className={playerView === "library" ? "on" : ""}
              onClick={() => setPlayerView("library")}
              role="button"
              tabIndex={0}
              aria-pressed={playerView === "library"}
              onKeyDown={(e) =>
                e.key === "Enter" || e.key === " " ? setPlayerView("library") : undefined
              }
            >
              Library
            </b>
            <b
              className={playerView === "deck" ? "on" : ""}
              onClick={() => setPlayerView("deck")}
              role="button"
              tabIndex={0}
              aria-pressed={playerView === "deck"}
              onKeyDown={(e) =>
                e.key === "Enter" || e.key === " " ? setPlayerView("deck") : undefined
              }
            >
              Now Playing
            </b>
          </div>
          {playerView === "library" && libSection === "albums" && (
            <span className="count">{count} albums</span>
          )}
          <div className="spacer" />
        </div>
        {playerView === "library" ? <LibraryView /> : <DeckShell />}
      </main>
      <TransportShell />
      <QueuePanel />
      {lyricsOpen && <LyricsPanel onClose={toggleLyrics} />}
    </>
  );
}
