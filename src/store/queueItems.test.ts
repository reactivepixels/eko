import { describe, it, expect } from "vitest";
import { toQueueItem, withQids } from "./queueItems";
import type { StreamSrcUrl, Track } from "../types";

const local: Track = {
  id: "l1",
  path: "/m/a.flac",
  title: "A",
  artist: "Ar",
  album: "Al",
  duration: 12.3456,
  bitrate: null,
  sampleRate: null,
  channels: 2,
  rgTrackGain: -6.5,
  rgTrackPeak: 0.9,
};

const server: Track = {
  id: "s1",
  subsonicId: "s1",
  path: "",
  title: null,
  artist: null,
  album: null,
  duration: 200,
  bitrate: null,
  sampleRate: null,
  channels: null,
  server: "https://music.example.com",
  coverUrl: "stream://localhost/?src=x",
  streamSrcUrl: "https://music.example.com/rest/stream?id=s1&u=rod&t=abc123&s=salt" as StreamSrcUrl,
};

describe("withQids", () => {
  it("gives the same song queued twice two different slot ids", () => {
    const [a, b] = withQids([server, server]);
    expect(a.qid).toBeTruthy();
    expect(a.qid).not.toBe(b.qid);
    expect(a.id).toBe(b.id);
  });

  it("never reuses a slot id", () => {
    const [a] = withQids([local]);
    const [b] = withQids([local]);
    expect(a.qid).not.toBe(b.qid);
  });
});

describe("toQueueItem", () => {
  it("sends a local file by path, with its ReplayGain tags and its length in ms", () => {
    const item = toQueueItem(withQids([local])[0]);
    expect(item.media).toEqual({ kind: "file", path: "/m/a.flac" });
    expect(item.durationMs).toBe(12346);
    expect(item.rg).toEqual({ trackGain: -6.5, trackPeak: 0.9, albumGain: null, albumPeak: null });
  });

  it("sends a server track as its id and server, never its signed URL", () => {
    const [t] = withQids([server]);
    const item = toQueueItem(t);
    expect(item.uid).toBe(t.qid);
    expect(item.media).toEqual({ kind: "remote", id: "s1", server: "https://music.example.com" });
    expect(JSON.stringify(item)).not.toContain("t=abc123");
    expect(item.title).toBe("Unknown");
    expect(item.coverUrl).toBe("stream://localhost/?src=x");
  });
});
