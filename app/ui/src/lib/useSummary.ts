import { useCallback, useEffect, useState } from "react";

import { api, errorText, type Recording, type Summary } from "./api";

type State =
  | { kind: "loading" }
  /** Nothing saved yet; the view offers to make one. */
  | { kind: "absent" }
  | { kind: "working" }
  | { kind: "ready"; summary: Summary }
  | { kind: "failed"; message: string };

/**
 * The saved summary for a recording, and the action that produces one.
 *
 * Never runs implicitly. A summary costs a full CLI invocation — the provider
 * re-pays its own startup context every call — so it happens only when asked.
 */
export function useSummary(recording: Recording | null, onSummarized?: () => void) {
  const [state, setState] = useState<State>({ kind: "loading" });

  useEffect(() => {
    if (!recording) return;
    let alive = true;
    setState({ kind: "loading" });

    void (async () => {
      try {
        const summary = await api.getSummary(recording.id);
        if (!alive) return;
        setState(summary ? { kind: "ready", summary } : { kind: "absent" });
      } catch {
        if (alive) setState({ kind: "absent" });
      }
    })();

    return () => {
      alive = false;
    };
  }, [recording]);

  const summarize = useCallback(async () => {
    if (!recording) return;
    setState({ kind: "working" });
    try {
      const summary = await api.summarize(recording.id);
      setState({ kind: "ready", summary });
      onSummarized?.();
    } catch (e) {
      setState({ kind: "failed", message: errorText(e) });
    }
  }, [recording, onSummarized]);

  return { state, summarize };
}
