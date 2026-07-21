/** `m:ss` for a duration in seconds. */
export function duration(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/**
 * The name a recording carries inside a space.
 *
 * A folder is named for when it was recorded, because that is what the user
 * remembers about a call. The directory on disk keeps its stable `session-<unix>`
 * id: renaming real folders to match a label would break every id already saved in
 * a transcript, a summary, or a settings file.
 */
export function folderName(unixSecs: number): string {
  if (!unixSecs) return "Recording";
  const date = new Date(unixSecs * 1000);
  const stamp = new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
  return `Recording - ${stamp}`;
}

/**
 * What a recording is called in a list: its user-given name, or the recorded-at
 * timestamp when it has none. The one place the fallback rule lives, so every list
 * agrees on what an unnamed recording reads as.
 */
export function recordingLabel(rec: { name: string | null; created: number }): string {
  return rec.name?.trim() || folderName(rec.created);
}

/** Coarse "how long ago", enough for a list; exact times live in the transcript. */
export function relativeTime(unixSecs: number): string {
  if (!unixSecs) return "";
  const diff = Math.floor(Date.now() / 1000) - unixSecs;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}
