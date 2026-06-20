import { useUiStore } from "../store/useUiStore";
import { useSubsonic } from "../subsonic/useSubsonic";
import { useLocal } from "../local/useLocal";
import { TopBar } from "./TopBar";
import { Sidebar } from "./Sidebar";
import { LibraryView } from "./LibraryView";
import { DeckView } from "./DeckView";
import { TransportBar } from "./TransportBar";
import { QueuePanel } from "./QueuePanel";
import "./neu.css";

export function PlayerApp() {
  const playerView = useUiStore((s) => s.playerView);
  const setPlayerView = useUiStore((s) => s.setPlayerView);
  const source = useUiStore((s) => s.source);
  const libSection = useUiStore((s) => s.libSection);
  const theme = useUiStore((s) => s.theme);
  const accent = useUiStore((s) => s.accent);
  const skin = useUiStore((s) => s.skin);
  const subCount = useSubsonic((s) => s.albums.length);
  const localCount = useLocal((s) => s.albums.length);
  const count = source === "server" ? subCount : localCount;

  // ONE app, themed. `data-skin` selects the theme layer (Porcelain | Studio) over the same
  // shell + views — never a separate app.
  return (
    <div className="app" data-theme={theme} data-accent={accent} data-skin={skin}>
      <TopBar />
      <Sidebar />
      <main className="main">
        <div className="main-head">
          <div className="viewseg">
            <b
              className={playerView === "library" ? "on" : ""}
              onClick={() => setPlayerView("library")}
            >
              Library
            </b>
            <b className={playerView === "deck" ? "on" : ""} onClick={() => setPlayerView("deck")}>
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
    </div>
  );
}
