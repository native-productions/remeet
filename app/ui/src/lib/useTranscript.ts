import { useCallback, useEffect, useState } from "react";

import { api, errorText, type Line, type Recording } from "./api";

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

  useEffect(() => {
    if (!recording) return;
    let alive = true;
    setState({ kind: "loading" });

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

  /** Runs transcription, replacing any cached transcript with the new result. */
  const transcribe = useCallback(async () => {
    if (!recording) return;
    setState({ kind: "working" });
    try {
      const lines = await api.transcribe(recording.id);
      setState({ kind: "ready", lines });
      onTranscribed?.();
    } catch (e) {
      setState({ kind: "failed", message: errorText(e) });
    }
  }, [recording, onTranscribed]);

  return { state, transcribe };
}
