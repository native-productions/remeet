import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  api,
  errorText,
  type Line,
  type Recording,
  type TranscribeSegment,
} from "./api";

type State =
  | { kind: "loading" }
  /** No transcript on disk yet; the view offers to make one. */
  | { kind: "absent" }
  | { kind: "working" }
  | { kind: "ready"; lines: Line[] }
  | { kind: "failed"; message: string };

/**
 * The transcript for one recording, and the action that produces it.
 *
 * Transcription is a separate, expensive step by design — the WAVs on disk are the
 * durable artifact — so this never runs it implicitly. Opening a recording only
 * reads what is already saved.
 */
export function useTranscript(recording: Recording | null, onTranscribed?: () => void) {
  const [state, setState] = useState<State>({ kind: "loading" });
  /** Segments streamed from the backend while a run is in progress, for a live
   *  preview. Arrives per track and unordered; the saved result is the truth. */
  const [live, setLive] = useState<Line[]>([]);

  useEffect(() => {
    if (!recording) return;
    let alive = true;
    setState({ kind: "loading" });
    setLive([]);

    void (async () => {
      try {
        const lines = await api.getTranscript(recording.id);
        if (!alive) return;
        setState(lines?.length ? { kind: "ready", lines } : { kind: "absent" });
      } catch {
        if (alive) setState({ kind: "absent" });
      }
    })();

    return () => {
      alive = false;
    };
  }, [recording]);

  /** Runs transcription, replacing any cached transcript with the new result.
   *
   *  While it runs, the backend streams each segment over `transcribe-segment`; those
   *  fill `live` so the view can show the transcript arriving rather than a bare
   *  spinner. The listener is scoped to this run and torn down when it ends. */
  const transcribe = useCallback(async () => {
    if (!recording) return;
    const id = recording.id;
    setLive([]);
    setState({ kind: "working" });

    const unlisten = await listen<TranscribeSegment>("transcribe-segment", (event) => {
      if (event.payload.id !== id) return;
      setLive((current) => [
        ...current,
        {
          speaker: event.payload.speaker,
          start_secs: event.payload.start_secs,
          text: event.payload.text,
        },
      ]);
    });

    try {
      const lines = await api.transcribe(id);
      setState({ kind: "ready", lines });
      onTranscribed?.();
    } catch (e) {
      setState({ kind: "failed", message: errorText(e) });
    } finally {
      unlisten();
      // The saved transcript now stands in for the preview.
      setLive([]);
    }
  }, [recording, onTranscribed]);

  return { state, live, transcribe };
}
