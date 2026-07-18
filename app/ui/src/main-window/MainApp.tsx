import { useCallback, useEffect, useState } from "react";

import { Player } from "../components/Player";
import { RecordingList } from "../components/RecordingList";
import { SpacePicker } from "../components/SpacePicker";
import { SummaryBody } from "../components/SummaryBody";
import { TranscriptBody } from "../components/TranscriptBody";
import { api, PROVIDERS, type Recording } from "../lib/api";
import { duration, relativeTime } from "../lib/format";
import { useAudioPlayer } from "../lib/useAudioPlayer";
import { useRecorder } from "../lib/useRecorder";
import { useRecordings } from "../lib/useRecordings";
import { useSpaces } from "../lib/useSpaces";
import { useSummary } from "../lib/useSummary";
import { useTranscript } from "../lib/useTranscript";
import { SettingsPane } from "./SettingsPane";
import { SpacesBrowser } from "./SpacesBrowser";

type Section = "recordings" | "spaces" | "settings";
type DetailTab = "transcript" | "summary";

/**
 * The main window: the workspace half of the app.
 *
 * The popover stays a capture surface — start, stop, glance. This is where there is
 * room to read, compare, and (as they land) organise recordings into projects.
 */
export function MainApp() {
  const [section, setSection] = useState<Section>("recordings");
  const [tab, setTab] = useState<DetailTab>("transcript");
  const [selected, setSelected] = useState<Recording | null>(null);
  const [providerLabel, setProviderLabel] = useState("the AI CLI");
  /** Which space is being browsed, and whether the browser is at its root. */
  const [openSpace, setOpenSpace] = useState<string | null>(null);
  const [atSpacesRoot, setAtSpacesRoot] = useState(true);

  const { recordings, refresh } = useRecordings();
  const { spaces, activeSpace, chooseActive, refresh: refreshSpaces } = useSpaces();
  const recorder = useRecorder(useCallback(() => void refresh(), [refresh]));

  const player = useAudioPlayer(selected?.id ?? null);
  const transcript = useTranscript(selected, refresh);
  const summary = useSummary(selected, refresh);

  // Which provider is configured decides what the summary view says it will run,
  // so it is re-read whenever the user comes back from Settings.
  useEffect(() => {
    if (section !== "recordings") return;
    void api
      .getSettings()
      .then((s) => {
        const found = PROVIDERS.find((p) => p.id === s.provider);
        if (found) setProviderLabel(found.label);
      })
      .catch(() => {});
  }, [section]);

  // The selected recording can be deleted out from under the detail pane, so the
  // selection is reconciled against the list that comes back, not the stale one.
  const onListChanged = useCallback(async () => {
    const fresh = await refresh();
    setSelected((current) =>
      current && fresh.some((r) => r.id === current.id) ? current : null,
    );
  }, [refresh]);

  const open = (recording: Recording) => {
    setSelected(recording);
    setTab("transcript");
  };

  const showsDetail = section === "recordings" || section === "spaces";

  return (
    <div className={`win${showsDetail ? "" : " is-wide"}`}>
      <aside className="side">
        <div className="side-head">
          <span className="wordmark">
            <i className="mark" aria-hidden="true" />
            Remeet
          </span>
        </div>

        <nav className="side-nav" aria-label="Sections">
          <button
            className={`side-item${section === "recordings" ? " is-active" : ""}`}
            type="button"
            onClick={() => setSection("recordings")}
          >
            Recordings
            <span className="side-count">{recordings.length}</span>
          </button>
          <button
            className={`side-item${section === "spaces" ? " is-active" : ""}`}
            type="button"
            onClick={() => {
              setSection("spaces");
              void refreshSpaces();
            }}
          >
            Spaces
            <span className="side-count">{spaces.length + 1}</span>
          </button>
          <button
            className={`side-item${section === "settings" ? " is-active" : ""}`}
            type="button"
            onClick={() => setSection("settings")}
          >
            Settings
          </button>
        </nav>

        <div className="side-foot">
          {/* Same control as the popover, same stored destination: wherever you
              start a recording, you choose where it lands from there. */}
          <div className="side-space">
            <span className="side-space-label">Save to</span>
            <SpacePicker
              spaces={spaces}
              value={activeSpace}
              disabled={recorder.recording}
              onChange={(id) => void chooseActive(id)}
            />
          </div>
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

      {section === "settings" ? (
        <SettingsPane />
      ) : (
        <>
          <section className="list-col">
            {section === "spaces" ? (
              <SpacesBrowser
                spaces={spaces}
                recordings={recordings}
                spaceId={openSpace}
                atRoot={atSpacesRoot}
                openedId={selected?.id ?? null}
                openedView={tab}
                onEnterSpace={(id) => {
                  setOpenSpace(id);
                  setAtSpacesRoot(false);
                }}
                onLeaveSpace={() => setAtSpacesRoot(true)}
                onOpen={({ recording, view }) => {
                  setSelected(recording);
                  setTab(view);
                }}
                onSpacesChanged={() => void refreshSpaces()}
              />
            ) : (
              <>
                <header className="col-head">
                  <h1 className="col-title">Recordings</h1>
                </header>
                <div className="col-body">
                  <RecordingList
                    recordings={recordings}
                    selectedId={selected?.id ?? null}
                    emptyTitle="No recordings yet"
                    emptySub="Hit Record in the sidebar, or use the menu-bar popover."
                    onOpen={open}
                    onChanged={onListChanged}
                  />
                </div>
              </>
            )}
          </section>

          <section className="detail-col">
            {selected ? (
              <>
                <header className="col-head detail-head">
                  <div className="tmeta">
                    <span className="ttitle">{duration(selected.duration_secs)}</span>
                    <span className="tsub">{relativeTime(selected.created)}</span>
                  </div>
                  <div className="dtabs" role="tablist" aria-label="Detail views">
                    <button
                      className={`dtab${tab === "transcript" ? " is-active" : ""}`}
                      type="button"
                      role="tab"
                      aria-selected={tab === "transcript"}
                      onClick={() => setTab("transcript")}
                    >
                      Transcript
                    </button>
                    <button
                      className={`dtab${tab === "summary" ? " is-active" : ""}`}
                      type="button"
                      role="tab"
                      aria-selected={tab === "summary"}
                      onClick={() => setTab("summary")}
                    >
                      Summary
                    </button>
                  </div>
                  {tab === "transcript" && transcript.state.kind === "ready" && (
                    <button
                      className="redo"
                      type="button"
                      onClick={() => void transcript.transcribe()}
                    >
                      Re-transcribe
                    </button>
                  )}
                </header>

                <Player player={player} />

                {tab === "transcript" ? (
                  <TranscriptBody
                    state={transcript.state}
                    onTranscribe={() => void transcript.transcribe()}
                  />
                ) : (
                  <SummaryBody
                    state={summary.state}
                    canSummarize={selected.transcribed}
                    providerLabel={providerLabel}
                    onSummarize={() => void summary.summarize()}
                  />
                )}
              </>
            ) : (
              <div className="empty">
                <p className="empty-title">Nothing selected</p>
                <p className="empty-sub">Pick a recording to read it back.</p>
              </div>
            )}
          </section>
        </>
      )}
    </div>
  );
}
