import { useCallback, useState } from "react";

import { Player } from "../components/Player";
import { RecordingList } from "../components/RecordingList";
import { TranscriptBody } from "../components/TranscriptBody";
import type { Recording } from "../lib/api";
import { duration, relativeTime } from "../lib/format";
import { useAudioPlayer } from "../lib/useAudioPlayer";
import { useRecorder } from "../lib/useRecorder";
import { useRecordings } from "../lib/useRecordings";
import { useTranscript } from "../lib/useTranscript";

/**
 * The main window: the workspace half of the app.
 *
 * The popover stays a capture surface — start, stop, glance. This is where there is
 * room to read, compare, and (as they land) organise recordings into projects. Only
 * Recordings exists today; the shell is arranged so the next section is an entry in
 * the sidebar rather than a rewrite.
 */
export function MainApp() {
  const [selected, setSelected] = useState<Recording | null>(null);

  const { recordings, refresh } = useRecordings();
  const recorder = useRecorder(useCallback(() => void refresh(), [refresh]));

  const player = useAudioPlayer(selected?.id ?? null);
  const { state, transcribe } = useTranscript(selected, refresh);

  // The selected recording can be deleted out from under the detail pane, so the
  // selection is reconciled against the list that comes back, not the stale one.
  const onListChanged = useCallback(async () => {
    const fresh = await refresh();
    setSelected((current) =>
      current && fresh.some((r) => r.id === current.id) ? current : null,
    );
  }, [refresh]);

  return (
    <div className="win">
      <aside className="side">
        <div className="side-head">
          <span className="wordmark">
            <i className="mark" aria-hidden="true" />
            Remeet
          </span>
        </div>

        <nav className="side-nav" aria-label="Sections">
          <button className="side-item is-active" type="button">
            Recordings
            <span className="side-count">{recordings.length}</span>
          </button>
        </nav>

        <div className="side-foot">
          <button
            className={`side-rec${recorder.recording ? " is-live" : ""}`}
            type="button"
            disabled={recorder.busy}
            onClick={() => void recorder.toggle()}
          >
            <span className="side-rec-dot" aria-hidden="true" />
            {recorder.recording ? `Stop · ${duration(recorder.elapsed)}` : "Record"}
          </button>
          {recorder.error && <p className="side-error">{recorder.error}</p>}
        </div>
      </aside>

      <section className="list-col">
        <header className="col-head">
          <h1 className="col-title">Recordings</h1>
        </header>
        <div className="col-body">
          <RecordingList
            recordings={recordings}
            selectedId={selected?.id ?? null}
            emptyTitle="No recordings yet"
            emptySub="Hit Record in the sidebar, or use the menu-bar popover."
            onOpen={setSelected}
            onChanged={onListChanged}
          />
        </div>
      </section>

      <section className="detail-col">
        {selected ? (
          <>
            <header className="col-head detail-head">
              <div className="tmeta">
                <span className="ttitle">{duration(selected.duration_secs)}</span>
                <span className="tsub">{relativeTime(selected.created)}</span>
              </div>
              {state.kind === "ready" && (
                <button className="redo" type="button" onClick={() => void transcribe()}>
                  Re-transcribe
                </button>
              )}
            </header>
            <Player player={player} />
            <TranscriptBody state={state} onTranscribe={() => void transcribe()} />
          </>
        ) : (
          <div className="empty">
            <p className="empty-title">Nothing selected</p>
            <p className="empty-sub">Pick a recording to read it back.</p>
          </div>
        )}
      </section>
    </div>
  );
}
