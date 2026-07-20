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
  summarized: boolean;
  /** Space id, or null for the default space. */
  space: string | null;
};

export type Space = {
  id: string;
  name: string;
  description: string;
  created: number;
};

/**
 * The default space is not a stored record: it is where a recording with no filing
 * belongs, which includes every recording made before spaces existed.
 */
export const DEFAULT_SPACE = {
  id: null,
  name: "Default Space",
  description: "Recordings that were not filed anywhere else.",
} as const;

export type Line = {
  speaker: "me" | "them";
  start_secs: number;
  text: string;
};

export type Status = {
  recording: boolean;
  elapsed_secs: number;
};

/** Which local CLI does the language work. Matches `remeet_ai::ProviderId`. */
export type ProviderId = "claude-code" | "codex";

export type ProviderConfig = {
  id: ProviderId;
  /** Explicit binary path, or null to use the name on PATH. */
  bin: string | null;
  /** Model to request, or null to let the CLI use its own default. */
  model: string | null;
};

export type Settings = {
  provider: ProviderId;
  claude_code: ProviderConfig;
  codex: ProviderConfig;
  /** Where the next recording is filed. Null means the default space. */
  active_space: string | null;
  /** Notify when another app has a call live, in case recording was forgotten. */
  call_reminder: boolean;
};

export type Probe = {
  installed: boolean;
  version: string | null;
  error: string | null;
};

export type Summary = {
  overview: string;
  key_points: string[];
  decisions: string[];
};

export const PROVIDERS: { id: ProviderId; label: string; bin: string }[] = [
  { id: "claude-code", label: "Claude Code", bin: "claude" },
  { id: "codex", label: "Codex", bin: "codex" },
];

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

  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  settingsPath: () => invoke<string>("settings_path"),
  /** Cheap: checks the binary runs. Says nothing about being logged in. */
  probeProvider: (provider: ProviderId) => invoke<Probe>("probe_provider", { provider }),
  /** Costs tokens: a real round trip, the only proof of login and model access. */
  testProvider: (provider: ProviderId) => invoke<string>("test_provider", { provider }),

  getSummary: (id: string) => invoke<Summary | null>("get_summary", { id }),
  summarize: (id: string) => invoke<Summary>("summarize", { id }),

  listSpaces: () => invoke<Space[]>("list_spaces"),
  createSpace: (name: string, description: string) =>
    invoke<Space>("create_space", { name, description }),
  renameSpace: (id: string, name: string, description: string) =>
    invoke<void>("rename_space", { id, name, description }),
  /** Removes the space only. Its recordings fall back to the default space. */
  deleteSpace: (id: string) => invoke<void>("delete_space", { id }),
  setActiveSpace: (space: string | null) => invoke<void>("set_active_space", { space }),
  moveRecording: (id: string, space: string | null) =>
    invoke<void>("move_recording", { id, space }),
};

/** Tauri errors arrive as plain strings, but a thrown value is still `unknown`. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
