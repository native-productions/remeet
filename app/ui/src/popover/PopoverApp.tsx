import { useCallback, useState } from "react";

import { Player } from "../components/Player";
import { RecordingList } from "../components/RecordingList";
import { SpacePicker } from "../components/SpacePicker";
import { TranscriptBody } from "../components/TranscriptBody";
import { api, type Recording } from "../lib/api";
import { duration, relativeTime } from "../lib/format";
import { useAudioPlayer } from "../lib/useAudioPlayer";
import { useRecorder } from "../lib/useRecorder";
import { useRecordings } from "../lib/useRecordings";
import { useSpaces } from "../lib/useSpaces";
import { useTranscript } from "../lib/useTranscript";

type Tab = "record" | "library";

/**
 * The menu-bar popover: one glance, one action.
 *
 * It stays deliberately small — record, list, read back. Anything that needs room
 * to think in belongs in the main window, which this can open.
 */
export function PopoverApp() {
  const [tab, setTab] = useState<Tab>("record");
  const [open, setOpen] = useState<Recording | null>(null);

  const { recordings, refresh } = useRecordings();
  const { spaces, activeSpace, chooseActive } = useSpaces();
  const recorder = useRecorder(
    // A finished recording is the thing you just made; show it.
    useCallback(() => {
      void refresh();
      setTab("library");
    }, [refresh]),
  );

  const player = useAudioPlayer(open?.id ?? null);
  const { state, transcribe } = useTranscript(open, refresh);

  const leaveTranscript = () => {
    setOpen(null);
    void refresh();
  };

  return (
    <div className={`app${recorder.recording ? " is-recording" : ""}`}>
      <header className="topbar">
        <span className="wordmark">
          <i className="mark" aria-hidden="true" />
          Remeet
        </span>
        <button
          className="expand"
          type="button"
          aria-label="Open the Remeet window"
          onClick={() => void api.openMainWindow()}
        >
          Open
        </button>
      </header>

      {open ? (
        <div className="thead">
          <button
            className="back"
            type="button"
            aria-label="Back to library"
            onClick={leaveTranscript}
          >
            <span aria-hidden="true">←</span>
          </button>
          <div className="tmeta">
            <span className="ttitle">{duration(open.duration_secs)}</span>
            <span className="tsub">{relativeTime(open.created)}</span>
          </div>
          {state.kind === "ready" && (
            // Re-runs transcription over the same audio, so a recording can pick
            // up transcription changes (e.g. bleed suppression).
            <button className="redo" type="button" onClick={() => void transcribe()}>
              Re-transcribe
            </button>
          )}
        </div>
      ) : (
        <nav className="tabs" role="tablist" aria-label="Views">
          <button
            className={`tab${tab === "record" ? " is-active" : ""}`}
            type="button"
            role="tab"
            aria-selected={tab === "record"}
            onClick={() => setTab("record")}
          >
            Record
          </button>
          <button
            className={`tab${tab === "library" ? " is-active" : ""}`}
            type="button"
            role="tab"
            aria-selected={tab === "library"}
            onClick={() => {
              setTab("library");
              void refresh();
            }}
          >
            Library
          </button>
        </nav>
      )}

      <main className="stack">
        {open ? (
          <section className="view transcript" role="tabpanel">
            <Player player={player} />
            <TranscriptBody state={state} onTranscribe={() => void transcribe()} />
          </section>
        ) : tab === "record" ? (
          <section className="view record" role="tabpanel">
            <div className="rec-wrap">
              <span className="rec-state">
                {recorder.error ?? (recorder.recording ? "Recording" : "Ready to record")}
              </span>
              <button
                className="rec"
                type="button"
                aria-label={recorder.recording ? "Stop recording" : "Start recording"}
                disabled={recorder.busy}
                onClick={() => void recorder.toggle()}
              >
                <span className="rec-core" aria-hidden="true" />
              </button>
              {recorder.recording && (
                <span className="rec-timer">{duration(recorder.elapsed)}</span>
              )}
            </div>
            {/* Filed before the fact: choosing where a call lands is part of
                starting it, and there is no filing step afterwards to forget. */}
            <div className="rec-space">
              <span className="rec-space-label">Save to</span>
              <SpacePicker
                spaces={spaces}
                value={activeSpace}
                disabled={recorder.recording}
                onChange={(id) => void chooseActive(id)}
              />
            </div>
            <p className="rec-hint">
              Records both sides of the call. Everything stays on this Mac.
            </p>
          </section>
        ) : (
          <section className="view library" role="tabpanel">
            <RecordingList
              recordings={recordings}
              emptyTitle="No recordings yet"
              emptySub="Head to the Record tab to capture your first meeting."
              onOpen={setOpen}
              onChanged={refresh}
            />
          </section>
        )}
      </main>
    </div>
  );
}
