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

/**
 * Branded URL types for the two pre-signed audio endpoints.
 *
 * These exist for exactly one reason: `downloadUrl` and `streamSrcUrl` are both `string`,
 * both point at the same server, and both play — but only `downloadUrl` (the Subsonic
 * `download` endpoint) returns the ORIGINAL bytes. `streamSrcUrl` is the `stream` endpoint,
 * which the server may transcode. Passing the wrong one to the Pro offline cache fills it
 * with transcoded audio while every track still downloads, plays and sounds correct. No
 * test catches it, and no runtime check can: by the time the bytes land they look fine.
 *
 * The brands make that substitution a **compile error** instead of a doc plea. They cost
 * nothing at runtime — the `__endpoint` property is phantom, never written and never read.
 * That works here because these values are only ever *produced* at a declaration-only
 * boundary (`invoke<SubSong>()`), where TypeScript simply believes the declared wire type,
 * so no call site needs a cast to create one.
 *
 * If you find yourself reaching for `as DownloadUrl` to silence an error: stop. That error
 * is the entire safeguard, and it is telling you the value came from the wrong endpoint.
 */
export type DownloadUrl = string & { readonly __endpoint: "download" };
export type StreamSrcUrl = string & { readonly __endpoint: "stream" };

// A playlist entry: metadata plus a stable id for list operations.
// Local tracks have a `path`; Subsonic tracks have a `subsonicId` (streamed instead).
export interface Track extends TrackMetadata {
  id: string;
  subsonicId?: string;
  mime?: string;
  coverArt?: string; // Subsonic cover-art id (for now-playing art)

  // Pre-signed URLs, carried straight through from the `SubSong` this track was built
  // from (server tracks only; a local file leaves all three undefined). They exist
  // because signing needs the server password, which now lives only in Rust — the
  // frontend can no longer mint a URL from an id, so the URL travels with the object.
  /** DIRECT upstream stream URL for the native engine. May be transcoded by the server. */
  streamSrcUrl?: StreamSrcUrl;
  /** DIRECT `download` URL — original bytes, no transcode (offline cache). */
  downloadUrl?: DownloadUrl;
  /** Proxied cover URL, **size-agnostic** — size it with `coverAt` from `nativeSubsonic`. */
  coverUrl?: string;
}

export type RepeatMode = "off" | "all" | "one";
