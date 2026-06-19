import { open } from "@tauri-apps/plugin-dialog";
import { readFile } from "@tauri-apps/plugin-fs";
import { invoke } from "@tauri-apps/api/core";
import type { Track, TrackMetadata } from "../types";

export const AUDIO_EXTENSIONS = ["mp3", "m4a", "aac", "wav", "aiff", "aif", "flac", "ogg", "opus"];

const MIME_BY_EXT: Record<string, string> = {
  mp3: "audio/mpeg",
  m4a: "audio/mp4",
  aac: "audio/aac",
  wav: "audio/wav",
  aiff: "audio/aiff",
  aif: "audio/aiff",
  flac: "audio/flac",
  ogg: "audio/ogg",
  opus: "audio/opus",
};

export function mimeForPath(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return MIME_BY_EXT[ext] ?? "audio/mpeg";
}

/** Show the native open dialog; returns selected absolute paths (possibly empty). */
export async function pickAudioFiles(): Promise<string[]> {
  const selection = await open({
    multiple: true,
    directory: false,
    filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }],
  });
  if (!selection) return [];
  return Array.isArray(selection) ? selection : [selection];
}

/** Read tags/stream info from Rust and wrap into a playlist Track with a stable id. */
export async function toTrack(path: string): Promise<Track> {
  const meta = await invoke<TrackMetadata>("read_metadata", { path });
  return {
    ...meta,
    id: crypto.randomUUID(),
    mime: mimeForPath(path),
    channels: meta.channels ?? 2,
  };
}

/** Read the raw bytes of a file for blob playback. */
export async function readBytes(path: string): Promise<Uint8Array> {
  return readFile(path);
}
