import { useUiStore } from "../store/useUiStore";
import { useMusicSource } from "../hooks/useMusicSource";
import { useNativeMenu, StudioApp } from "@pro";
import { TopBar } from "./TopBar";
import { Sidebar } from "./Sidebar";
import { LibraryView } from "./LibraryView";
import { DeckView } from "./DeckView";
import { TransportBar } from "./TransportBar";
import { QueuePanel } from "./QueuePanel";
import { LyricsPanel } from "./LyricsPanel";
import "./neu.css";

export function PlayerApp() {
  const playerView = useUiStore((s) => s.playerView);
  const setPlayerView = useUiStore((s) => s.setPlayerView);
  const libSection = useUiStore((s) => s.libSection);
  const theme = useUiStore((s) => s.theme);
  const accent = useUiStore((s) => s.accent);
  const skin = useUiStore((s) => s.skin);
  const lyricsOpen = useUiStore((s) => s.lyricsOpen);
  const toggleLyrics = useUiStore((s) => s.toggleLyrics);
  const count = useMusicSource().albumCount;
  useNativeMenu(); // Pro: bridges the native "Skins" menu ↔ the UI store; no-op in free.

  // ONE app, themed. `data-skin` selects the theme layer (Porcelain | Studio) over the same
  // shell + views — never a separate app.
  //
  // StudioApp (Pro): when skin === "studio" it renders the Studio layout entirely,
  // ignoring children. When skin !== "studio" (or in the free build) it renders
  // children — the standard Porcelain layout below.
  return (
    <div className="app" data-theme={theme} data-accent={accent} data-skin={skin}>
      <StudioApp>
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
          {playerView === "library" ? <LibraryView /> : <DeckView />}
        </main>
        <TransportBar />
        <QueuePanel />
        {lyricsOpen && <LyricsPanel onClose={toggleLyrics} />}
      </StudioApp>
    </div>
  );
}
