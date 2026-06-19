import { create } from "zustand";
import {
  setConfig,
  ping,
  getAlbums,
  getAlbum,
  getRandomSongs,
  search,
  mimeForSong,
  getPlaylists,
  getPlaylist,
  type SubsonicConfig,
  type SubAlbum,
  type SubSong,
  type SubPlaylist,
} from "./client";
import { usePlayerStore } from "../store/usePlayerStore";
import type { Track } from "../types";

function toTrack(s: SubSong): Track {
  return {
    id: s.id,
    subsonicId: s.id,
    path: "",
    title: s.title ?? null,
    artist: s.artist ?? null,
    album: s.album ?? null,
    duration: s.duration ?? 0,
    bitrate: s.bitRate ?? null,
    sampleRate: s.samplingRate ?? null,
    channels: s.channelCount ?? 2,
    mime: mimeForSong(s),
    coverArt: s.coverArt,
  };
}

interface SubsonicState {
  connected: boolean;
  status: "idle" | "connecting" | "error";
  error: string | null;
  config: SubsonicConfig | null;
  albums: SubAlbum[];
  playlists: SubPlaylist[];

  connect: (cfg: SubsonicConfig) => Promise<boolean>;
  autoConnect: () => Promise<void>;
  disconnect: () => void;
  playAlbum: (id: string) => Promise<void>;
  openAlbum: (id: string) => Promise<{ album: SubAlbum; tracks: Track[] }>;
  openPlaylist: (id: string) => Promise<{ name: string; tracks: Track[] }>;
  playTracks: (tracks: Track[], index: number) => void;
  loadRandom: () => Promise<void>;
  doSearch: (q: string) => Promise<{ albums: SubAlbum[]; songs: SubSong[] }>;
  queueSongs: (songs: SubSong[], autoplay?: boolean) => void;
}

export const useSubsonic = create<SubsonicState>((set, get) => ({
  connected: false,
  status: "idle",
  error: null,
  config: null,
  albums: [],
  playlists: [],

  connect: async (cfg) => {
    set({ status: "connecting", error: null });
    setConfig(cfg);
    try {
      await ping();
      const albums = await getAlbums(500);
      set({ connected: true, status: "idle", config: cfg, albums, error: null });
      try {
        localStorage.setItem("eko.subsonic", JSON.stringify(cfg));
      } catch {
        /* ignore */
      }
      // No auto-dump: the Track Library opens on the album browser, the user picks.
      getPlaylists()
        .then((playlists) => set({ playlists }))
        .catch(() => {
          /* ignore */
        });
      return true;
    } catch (e) {
      setConfig(null);
      set({ connected: false, status: "error", error: String(e instanceof Error ? e.message : e) });
      return false;
    }
  },

  autoConnect: async () => {
    let raw: string | null = null;
    try {
      raw = localStorage.getItem("eko.subsonic");
    } catch {
      /* ignore */
    }
    if (!raw) return;
    try {
      await get().connect(JSON.parse(raw) as SubsonicConfig);
    } catch {
      /* show panel */
    }
  },

  disconnect: () => {
    setConfig(null);
    try {
      localStorage.removeItem("eko.subsonic");
    } catch {
      /* ignore */
    }
    set({ connected: false, status: "idle", config: null, albums: [] });
  },

  playAlbum: async (id) => {
    const { songs } = await getAlbum(id);
    usePlayerStore.getState().setQueue(songs.map(toTrack), true);
  },

  openAlbum: async (id) => {
    const { album, songs } = await getAlbum(id);
    return { album, tracks: songs.map(toTrack) };
  },

  openPlaylist: async (id) => {
    const { name, songs } = await getPlaylist(id);
    return { name, tracks: songs.map(toTrack) };
  },

  playTracks: (tracks, index) => {
    const p = usePlayerStore.getState();
    p.setQueue(tracks, false);
    void p.playAt(index);
  },

  loadRandom: async () => {
    const songs = await getRandomSongs(50);
    usePlayerStore.getState().setQueue(songs.map(toTrack), false);
  },

  doSearch: async (q) => search(q),

  queueSongs: (songs, autoplay = false) => {
    usePlayerStore.getState().setQueue(songs.map(toTrack), autoplay);
  },
}));
