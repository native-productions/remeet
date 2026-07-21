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
  /** User-given label, or null to fall back to the recorded-at timestamp. */
  name: string | null;
};

/**
 * One segment streamed from the backend while a transcription runs, for the live
 * preview. Carries the recording id so a stale run's events can be ignored.
 */
export type TranscribeSegment = {
  id: string;
  speaker: "me" | "them";
  start_secs: number;
  text: string;
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
  /** Recording, but capture is paused. */
  paused: boolean;
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
  /** Transcription speed/accuracy trade-off. */
  transcribe_speed: "accurate" | "fast";
  /** Forced transcription language (ISO code), or null to auto-detect. */
  transcribe_language: string | null;
  /** Suppress background noise on the microphone before transcribing. */
  mic_denoise: boolean;
  /** Which engine transcribes: built-in whisper.cpp, or the external whisper CLI. */
  transcribe_engine: "builtin" | "whisper-cli";
  /** External whisper tool location and model, used when the engine is the CLI. */
  whisper_cli: { bin: string; model: string };
  /** GGML model the built-in engine loads (resolved to ~/whisper/models/ggml-<model>.bin). */
  whisper_builtin: { model: string };
};

/** Build identity shown in the UI: the version, and whether this is a dev run. */
export type AppInfo = {
  version: string;
  /** True for a `bun run app` terminal build, false for the installed bundle. */
  dev: boolean;
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
  /** Version + dev/release mode, for the version line and DEV badge. */
  appInfo: () => invoke<AppInfo>("app_info"),
  getStatus: () => invoke<Status>("get_status"),
  listRecordings: () => invoke<Recording[]>("list_recordings"),
  startRecording: () => invoke<void>("start_recording"),
  /** Freezes capture without ending the session; audio resumes gap-free. */
  pauseRecording: () => invoke<void>("pause_recording"),
  resumeRecording: () => invoke<void>("resume_recording"),
  stopRecording: () => invoke<Recording>("stop_recording"),
  getTranscript: (id: string) => invoke<Line[] | null>("get_transcript", { id }),
  transcribe: (id: string) => invoke<Line[]>("transcribe", { id }),
  /** Asks the in-flight transcription to stop; the `transcribe` call then rejects. */
  cancelTranscribe: () => invoke<void>("cancel_transcribe"),
  /** Builds if needed and returns the recording's playback mix path. */
  prepareAudio: (id: string) => invoke<string>("prepare_audio", { id }),
  /** Permanent: removes the audio, the mixdown, and the transcript. */
  deleteRecording: (id: string) => invoke<void>("delete_recording", { id }),
  /** Opens the recording's folder in Finder so the raw files can be reached. */
  revealRecording: (id: string) => invoke<void>("reveal_recording", { id }),
  openMainWindow: () => invoke<void>("open_main_window"),

  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  settingsPath: () => invoke<string>("settings_path"),
  /** Best-effort path to the external `whisper` tool, or null if not found. */
  detectWhisper: () => invoke<string | null>("detect_whisper"),
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
  /** Sets a recording's label; null (or blank) clears it back to the timestamp. */
  renameRecording: (id: string, name: string | null) =>
    invoke<void>("rename_recording", { id, name }),
};

/** Tauri errors arrive as plain strings, but a thrown value is still `unknown`. */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
