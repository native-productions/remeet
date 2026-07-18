// Typed wrappers over the Tauri commands in `app/src-tauri/src/commands.rs`.
//
// Every IPC call goes through here so the shapes the Rust side serializes are
// declared in exactly one place, rather than being re-guessed at each call site.

import { invoke } from "@tauri-apps/api/core";

export type Recording = {
  /** Directory name under the recordings root; the id for every other command. */
  id: string;
  duration_secs: number;
  /** Unix seconds, for sorting and relative time. */
  created: number;
  transcribed: boolean;
};

export type Line = {
  speaker: "me" | "them";
  start_secs: number;
  text: string;
};

export type Status = {
  recording: boolean;
  elapsed_secs: number;
};

export const api = {
  getStatus: () => invoke<Status>("get_status"),
  listRecordings: () => invoke<Recording[]>("list_recordings"),
  startRecording: () => invoke<void>("start_recording"),
  stopRecording: () => invoke<Recording>("stop_recording"),
  getTranscript: (id: string) => invoke<Line[] | null>("get_transcript", { id }),
  transcribe: (id: string) => invoke<Line[]>("transcribe", { id }),
  /** Builds the playback mixdown if needed; returns its path on disk. */
  prepareAudio: (id: string) => invoke<string>("prepare_audio", { id }),
  /** Permanent: removes the audio, the mixdown, and the transcript. */
  deleteRecording: (id: string) => invoke<void>("delete_recording", { id }),
  openMainWindow: () => invoke<void>("open_main_window"),
};

/** Tauri errors arrive as plain strings, but a thrown value is still `unknown`. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
