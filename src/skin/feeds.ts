/**
 * Phase 1 · Feed catalog — the stable engine/state contract.
 * See docs/skin-architecture.md §3 (Palette / Chassis / Layout) and §4 (primitives).
 *
 * A "feed" is a canonical binding into the player's engine + UI state. It is the contract
 * that makes a component portable: a control speaks to a *feed*, never to a store shape or a
 * theme. Parity lives here — Porcelain's faders and Studio's knobs both bind the `bandGains`
 * feed; that's why they're interchangeable surfaces onto the one engine EQ.
 *
 * This file declares WHAT the feeds are (id, kind, value, where they live). The React hooks
 * that actually read/write them are wired in Phase 2 with the primitives — keeping the
 * catalog as *data* lets the registry, manifest and (later) the builder reason about feeds
 * without importing any component.
 *
 * HONESTY RULE: `derived` feeds (e.g. `bitPerfect`) are read-only status, never fake
 * toggles. A control may only WRITE a feed whose kind allows it.
 */

export type FeedKind =
  | "read" // UI reads engine/state (e.g. currentTrack)
  | "write" // momentary action, no readable value (e.g. skipNext)
  | "readwrite" // two-way control (e.g. volume)
  | "derived" // computed, READ-ONLY status (e.g. bitPerfect) — never a fake toggle
  | "stream" // continuously sampled (e.g. spectrum bands)
  | "list"; // a catalogue/collection (e.g. eqPresets)

export type FeedCategory =
  | "transport"
  | "output"
  | "tone"
  | "track"
  | "signal"
  | "stream"
  | "library"
  | "appearance";

interface FeedDescriptor {
  /** Stable id a component binds to. */
  id: string;
  label: string;
  kind: FeedKind;
  /** Human/TS description of the value carried (documentation, not enforced here). */
  value: string;
  /** Where it lives today — store field + action. Documents the Phase-2 binding target. */
  binding: string;
  category: FeedCategory;
}

/**
 * The catalog. `FeedId` is derived from this array (below), so this is the single source of
 * truth — add a feed here and the type updates. Grounded in the real stores:
 * `usePlayerStore` (engine) and `useUiStore` (UI/appearance).
 */
export const FEEDS = [
  // ── transport ───────────────────────────────────────────────────────────────
  {
    id: "playPause",
    label: "Play / Pause",
    kind: "readwrite",
    value: "boolean (isPlaying)",
    binding: "playerStore.isPlaying / togglePlay()",
    category: "transport",
  },
  {
    id: "skipNext",
    label: "Next",
    kind: "write",
    value: "—",
    binding: "playerStore.next()",
    category: "transport",
  },
  {
    id: "skipPrev",
    label: "Previous",
    kind: "write",
    value: "—",
    binding: "playerStore.prev()",
    category: "transport",
  },
  {
    id: "position",
    label: "Position / Seek",
    kind: "readwrite",
    value: "{ currentTime, duration } seconds",
    binding: "playerStore.currentTime/duration / seek(),beginScrub,scrubMove,endScrub",
    category: "transport",
  },
  {
    id: "repeat",
    label: "Repeat",
    kind: "readwrite",
    value: '"off" | "all" | "one"',
    binding: "playerStore.repeat / cycleRepeat()",
    category: "transport",
  },
  {
    id: "shuffle",
    label: "Shuffle",
    kind: "readwrite",
    value: "boolean",
    binding: "playerStore.shuffle / toggleShuffle()",
    category: "transport",
  },

  // ── output ──────────────────────────────────────────────────────────────────
  {
    id: "volume",
    label: "Volume",
    kind: "readwrite",
    value: "number 0..1",
    binding: "playerStore.volume / setVolume()",
    category: "output",
  },
  {
    id: "outputDevice",
    label: "Output device",
    kind: "readwrite",
    value: "string | null (DAC name; null = system default)",
    binding: "playerStore.outputDevice / setOutputDevice()",
    category: "output",
  },

  // ── tone / EQ ───────────────────────────────────────────────────────────────
  {
    id: "eqEnabled",
    label: "EQ enabled",
    kind: "readwrite",
    value: "boolean",
    binding: "playerStore.eqEnabled / setEqEnabled()",
    category: "tone",
  },
  {
    id: "preamp",
    label: "Preamp",
    kind: "readwrite",
    value: "number (dB)",
    binding: "playerStore.preamp / setPreamp()",
    category: "tone",
  },
  {
    id: "bandGains",
    label: "EQ band gains",
    kind: "readwrite",
    value: "number[] (dB, length EQ_BAND_COUNT)",
    binding: "playerStore.gains / setBandGain(i,db), setAllGains()",
    category: "tone",
  },
  {
    id: "eqPreset",
    label: "EQ preset",
    kind: "readwrite",
    value: "string | null (presetName; null = Custom)",
    binding: "playerStore.presetName / applyPreset()",
    category: "tone",
  },
  {
    id: "eqPresets",
    label: "EQ preset catalogue",
    kind: "list",
    value: "EqPreset[]",
    binding: "constants.EQ_PRESETS",
    category: "tone",
  },
  {
    id: "eqBands",
    label: "EQ band frequencies",
    kind: "list",
    value: "readonly number[] (Hz centres)",
    binding: "constants.EQ_BANDS",
    category: "tone",
  },

  // ── current track / queue ──────────────────────────────────────────────────
  {
    id: "currentTrack",
    label: "Current track",
    kind: "read",
    value: "Track | null (title, artist, album, coverArt, duration…)",
    binding: "playerStore.tracks[currentIndex]",
    category: "track",
  },
  {
    id: "queue",
    label: "Queue",
    kind: "read",
    value: "{ tracks: Track[], currentIndex: number | null }",
    binding: "playerStore.tracks / currentIndex (+ reorder, removeTrack, playAt)",
    category: "track",
  },

  // ── signal path (the honest seal) ──────────────────────────────────────────
  {
    id: "engineInfo",
    label: "Signal path",
    kind: "read",
    value: "EngineInfo | null (device, rate, srcRate, devRate, bits, codec, channels)",
    binding: "playerStore.engineInfo",
    category: "signal",
  },
  {
    id: "bitPerfect",
    label: "Bit-perfect (locked)",
    kind: "derived",
    value: "boolean — EQ flat ∧ volume==1 ∧ devRate==srcRate ∧ no ReplayGain",
    binding: "derived from engineInfo + eqEnabled/gains + volume + rgAppliedDb",
    category: "signal",
  },
  {
    id: "replayGain",
    label: "ReplayGain",
    kind: "readwrite",
    value: '"off" | "track" | "album"  (+ applied dB read via rgAppliedDb)',
    binding: "playerStore.replayGainMode / setReplayGainMode(); rgAppliedDb (read)",
    category: "signal",
  },

  // ── stream (sampled) ───────────────────────────────────────────────────────
  {
    id: "spectrum",
    label: "Spectrum / level",
    kind: "stream",
    value: "number[] (32 band magnitudes, polled from Rust)",
    binding: "nativeEngine.startBands()/bands poller",
    category: "stream",
  },

  // ── library / navigation ───────────────────────────────────────────────────
  {
    id: "source",
    label: "Source",
    kind: "readwrite",
    value: '"server" | "local"',
    binding: "uiStore.source / setSource()",
    category: "library",
  },
  {
    id: "playerView",
    label: "View",
    kind: "readwrite",
    value: '"library" | "deck"',
    binding: "uiStore.playerView / setPlayerView()",
    category: "library",
  },
  {
    id: "libSection",
    label: "Library section",
    kind: "readwrite",
    value: '"albums" | "artists" | "tracks" | "folders" | "playlists"',
    binding: "uiStore.libSection / setLibSection()",
    category: "library",
  },
  {
    id: "librarySort",
    label: "Library sort",
    kind: "readwrite",
    value: '"name" | "artist" | "year"',
    binding: "uiStore.librarySort / setLibrarySort()",
    category: "library",
  },
  {
    id: "query",
    label: "Search",
    kind: "readwrite",
    value: "string",
    binding: "uiStore.query / setQuery()",
    category: "library",
  },

  // ── appearance (PALETTE + CHASSIS selectors — orthogonal to layout) ─────────
  {
    id: "theme",
    label: "Theme (light/dark)",
    kind: "readwrite",
    value: '"light" | "dark"  — palette',
    binding: "uiStore.theme / toggleTheme()",
    category: "appearance",
  },
  {
    id: "accent",
    label: "Accent",
    kind: "readwrite",
    value: "Accent  — palette",
    binding: "uiStore.accent / setAccent()  (ACCENTS catalogue)",
    category: "appearance",
  },
  {
    id: "chassis",
    label: "Chassis (skin)",
    kind: "readwrite",
    value: 'Skin  — chassis selector (codebase "skin" == architecture "chassis")',
    binding: "uiStore.skin / setSkin()  (SKINS catalogue)",
    category: "appearance",
  },
] as const satisfies readonly FeedDescriptor[];

/** Every valid feed id — derived from the catalog, so it can never drift from `FEEDS`. */
export type FeedId = (typeof FEEDS)[number]["id"];

/** Look up a feed descriptor by id. */
export const FEED_BY_ID: Readonly<Record<FeedId, FeedDescriptor>> = Object.fromEntries(
  FEEDS.map((f) => [f.id, f] as [FeedId, FeedDescriptor]),
) as Record<FeedId, FeedDescriptor>;

/** Feeds a component may WRITE (kind write/readwrite). Derived feeds are excluded by design. */
export function isWritable(id: FeedId): boolean {
  const k = FEED_BY_ID[id].kind;
  return k === "write" || k === "readwrite";
}
