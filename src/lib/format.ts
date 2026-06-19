/** Format seconds as M:SS (Winamp-style). Negative values are shown with a leading -. */
export function formatTime(seconds: number): string {
  const neg = seconds < 0;
  const s = Math.floor(Math.abs(seconds));
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return `${neg ? "-" : ""}${m}:${rem.toString().padStart(2, "0")}`;
}

/** "Artist - Title", falling back to title or the file name. */
export function trackLabel(t: {
  title: string | null;
  artist: string | null;
  path: string;
}): string {
  if (t.artist && t.title) return `${t.artist} - ${t.title}`;
  if (t.title) return t.title;
  return t.path.split("/").pop() ?? t.path;
}
