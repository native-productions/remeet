import { useCallback, useEffect, useRef, useState } from "react";
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
  /** Set while a cancel is in flight, so the rejected `transcribe` call can tell a
   *  user cancellation apart from a real failure. A ref, not state: it is read inside
   *  the same async closure and must not wait for a re-render. */
  const cancelled = useRef(false);

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
    cancelled.current = false;
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
      if (cancelled.current) {
        // A cancelled run leaves any prior transcript on disk untouched, so restore it
        // rather than showing an error.
        const prior = await api.getTranscript(id).catch(() => null);
        setState(prior?.length ? { kind: "ready", lines: prior } : { kind: "absent" });
      } else {
        setState({ kind: "failed", message: errorText(e) });
      }
    } finally {
      unlisten();
      // The saved transcript now stands in for the preview.
      setLive([]);
    }
  }, [recording, onTranscribed]);

  /** Asks the backend to stop the run. The `transcribe` promise then rejects, and its
   *  catch — seeing `cancelled` — restores the prior transcript instead of failing. */
  const cancel = useCallback(async () => {
    cancelled.current = true;
    try {
      await api.cancelTranscribe();
    } catch {
      // Best-effort: the run may already be ending on its own.
    }
  }, []);

  return { state, live, transcribe, cancel };
}
