import { useCallback, useEffect, useState } from "react";

import { api, errorText } from "./api";

/**
 * Recording state, polled from the Rust side once a second.
 *
 * The backend owns the truth: a session survives the popover closing, and both
 * windows can be open at once, so neither can hold the state locally.
 */
export function useRecorder(onStopped?: () => void) {
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;

    const poll = async () => {
      try {
        const status = await api.getStatus();
        if (!alive) return;
        setRecording(status.recording);
        setElapsed(status.elapsed_secs);
      } catch {
        // A failed poll is transient; leave the last known state in place.
      }
    };

    void poll();
    const timer = setInterval(poll, 1000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);

  const toggle = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (recording) {
        await api.stopRecording();
        setRecording(false);
        setElapsed(0);
        onStopped?.();
      } else {
        await api.startRecording();
        setRecording(true);
        setElapsed(0);
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }, [busy, recording, onStopped]);

  return { recording, elapsed, busy, error, toggle };
}
