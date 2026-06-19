// Mirror of the Rust `TrackMetadata` struct (metadata.rs). Field names are camelCase
// to match the `#[serde(rename_all = "camelCase")]` on the Rust side.
export interface TrackMetadata {
  path: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  duration: number; // seconds
  bitrate: number | null; // kbps
  sampleRate: number | null; // Hz
  channels: number | null; // 1 = mono, 2 = stereo
  // ReplayGain tags (local files; optional — server tracks may not carry them). Gains in dB,
  // peaks linear. Used by the optional, off-by-default volume normalisation.
  rgTrackGain?: number | null;
  rgAlbumGain?: number | null;
  rgTrackPeak?: number | null;
  rgAlbumPeak?: number | null;
}

export type ReplayGainMode = "off" | "track" | "album";

// A playlist entry: metadata plus a stable id for list operations.
// Local tracks have a `path`; Subsonic tracks have a `subsonicId` (streamed instead).
export interface Track extends TrackMetadata {
  id: string;
  subsonicId?: string;
  mime?: string;
  coverArt?: string; // Subsonic cover-art id (for now-playing art)
}

export type RepeatMode = "off" | "all" | "one";
