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
  const [paused, setPaused] = useState(false);
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
        setPaused(status.paused);
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
        setPaused(false);
        setElapsed(0);
        onStopped?.();
      } else {
        await api.startRecording();
        setRecording(true);
        setPaused(false);
        setElapsed(0);
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }, [busy, recording, onStopped]);

  // Pause/resume never touch the session slot, so they need no `busy` guard against
  // toggle — the backend commands are idempotent. Optimistic local flip keeps the
  // button responsive between one-second polls.
  const togglePause = useCallback(async () => {
    if (!recording) return;
    const next = !paused;
    setPaused(next);
    try {
      if (next) await api.pauseRecording();
      else await api.resumeRecording();
    } catch (e) {
      setPaused(!next);
      setError(errorText(e));
    }
  }, [recording, paused]);

  return { recording, paused, elapsed, busy, error, toggle, togglePause };
}
