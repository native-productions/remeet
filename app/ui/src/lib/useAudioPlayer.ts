import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";

import { api, errorText } from "./api";

/** The speeds worth having for spoken audio: normal, brisk, and skim. */
export const SPEEDS = [1, 1.5, 2] as const;

export type Player = ReturnType<typeof useAudioPlayer>;

/**
 * Playback for one recording.
 *
 * The audio element is kept in a ref rather than in the tree: it outlives renders,
 * and nothing about it belongs in the DOM the user sees.
 *
 * The file is loaded over the asset protocol (`convertFileSrc`), not as a blob.
 * WKWebView plays media through range requests, which the asset protocol serves and
 * a blob URL does not — with a blob the element simply never plays or seeks.
 */
export function useAudioPlayer(recordingId: string | null) {
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const loadedIdRef = useRef<string | null>(null);

  const [playing, setPlaying] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [position, setPosition] = useState(0);
  const [total, setTotal] = useState(0);
  const [speedIndex, setSpeedIndex] = useState(0);

  if (audioRef.current === null) {
    const audio = new Audio();
    audio.preload = "none";
    // Keeps voices sounding like voices at 1.5x and 2x instead of chipmunked.
    audio.preservesPitch = true;
    audioRef.current = audio;
  }
  const audio = audioRef.current;

  useEffect(() => {
    const onTime = () => setPosition(audio.currentTime);
    const onMeta = () =>
      setTotal(Number.isFinite(audio.duration) ? audio.duration : 0);
    const onPlay = () => setPlaying(true);
    const onPause = () => setPlaying(false);
    const onEnded = () => {
      setPlaying(false);
      audio.currentTime = 0;
      setPosition(0);
    };
    const onError = () =>
      setError(`audio failed to load (code ${audio.error?.code ?? "unknown"})`);

    audio.addEventListener("timeupdate", onTime);
    audio.addEventListener("loadedmetadata", onMeta);
    audio.addEventListener("play", onPlay);
    audio.addEventListener("pause", onPause);
    audio.addEventListener("ended", onEnded);
    audio.addEventListener("error", onError);

    return () => {
      audio.removeEventListener("timeupdate", onTime);
      audio.removeEventListener("loadedmetadata", onMeta);
      audio.removeEventListener("play", onPlay);
      audio.removeEventListener("pause", onPause);
      audio.removeEventListener("ended", onEnded);
      audio.removeEventListener("error", onError);
      audio.pause();
    };
  }, [audio]);

  // Switching recordings — or leaving the detail view — must stop whatever is
  // playing, so audio never continues under an unrelated screen.
  useEffect(() => {
    audio.pause();
    audio.removeAttribute("src");
    loadedIdRef.current = null;
    setPlaying(false);
    setError(null);
    setPosition(0);
    setTotal(0);
  }, [audio, recordingId]);

  const toggle = useCallback(async () => {
    if (!recordingId) return;
    if (!audio.paused) {
      audio.pause();
      return;
    }

    if (loadedIdRef.current !== recordingId) {
      if (loading) return;
      setLoading(true);
      setError(null);
      try {
        // The playback mix is built on first play rather than every time a recording
        // is opened, then cached next to the tracks.
        const path = await api.prepareAudio(recordingId);
        audio.src = convertFileSrc(path);
        audio.playbackRate = SPEEDS[speedIndex] ?? 1;
        loadedIdRef.current = recordingId;
      } catch (e) {
        setError(errorText(e));
        return;
      } finally {
        setLoading(false);
      }
    }

    try {
      await audio.play();
    } catch (e) {
      setError(errorText(e));
    }
  }, [audio, loading, recordingId, speedIndex]);

  const cycleSpeed = useCallback(() => {
    setSpeedIndex((i) => {
      const next = (i + 1) % SPEEDS.length;
      audio.playbackRate = SPEEDS[next] ?? 1;
      return next;
    });
  }, [audio]);

  const seek = useCallback(
    (seconds: number) => {
      if (!Number.isFinite(audio.duration)) return;
      audio.currentTime = seconds;
      setPosition(seconds);
    },
    [audio],
  );

  return {
    playing,
    loading,
    error,
    position,
    total,
    speed: SPEEDS[speedIndex] ?? 1,
    toggle,
    cycleSpeed,
    seek,
  };
}
