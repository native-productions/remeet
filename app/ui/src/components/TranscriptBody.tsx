import { useEffect, useRef } from "react";

import type { Line } from "../lib/api";
import { duration } from "../lib/format";

type State =
  | { kind: "loading" }
  | { kind: "absent" }
  | { kind: "working" }
  | { kind: "ready"; lines: Line[] }
  | { kind: "failed"; message: string };

type Props = {
  state: State;
  /** Segments arriving live while `state` is `working`; empty otherwise. */
  live?: Line[];
  onTranscribe: () => void;
};

/** One rendered transcript line, shared by the final view and the live preview. */
function TranscriptLine({ line, index }: { line: Line; index: number }) {
  return (
    <div
      // Lines have no id of their own; position is the only stable identity, and the
      // list is only ever appended to or replaced wholesale.
      key={index}
      className={`line ${line.speaker === "me" ? "me" : "them"}`}
    >
      <div className="line-tag">{line.speaker}</div>
      <div>
        <div className="line-text">{line.text}</div>
        <div className="line-time">{duration(line.start_secs)}</div>
      </div>
    </div>
  );
}

/** The live preview: transcript lines as they stream in, pinned to the newest. */
function LiveFeed({ lines }: { lines: Line[] }) {
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => {
    end.current?.scrollIntoView({ block: "end" });
  }, [lines.length]);

  return (
    <div className="live-feed">
      <div className="live-head">
        <span className="spin" />
        Transcribing on this Mac — {lines.length} segment
        {lines.length === 1 ? "" : "s"} so far
      </div>
      {lines.map((line, i) => (
        <TranscriptLine key={i} line={line} index={i} />
      ))}
      <div ref={end} />
    </div>
  );
}

/** The transcript itself, or whatever stands in for it while there isn't one. */
export function TranscriptBody({ state, live = [], onTranscribe }: Props) {
  switch (state.kind) {
    case "loading":
      return <div className="tbody" />;

    case "ready":
      return (
        <div className="tbody">
          {state.lines.map((line, i) => (
            <TranscriptLine key={i} line={line} index={i} />
          ))}
        </div>
      );

    case "working":
      return (
        <div className="tbody">
          {live.length > 0 ? (
            <LiveFeed lines={live} />
          ) : (
            <div className="cta">
              <div className="working">
                <span className="spin" />
                Transcribing on this Mac
              </div>
            </div>
          )}
        </div>
      );

    case "failed":
      return (
        <div className="tbody">
          <div className="cta">
            <p className="error">{state.message}</p>
            <button className="cta-btn" type="button" onClick={onTranscribe}>
              Try again
            </button>
          </div>
        </div>
      );

    case "absent":
      return (
        <div className="tbody">
          <div className="cta">
            <p className="cta-text">This recording has not been transcribed yet.</p>
            <button className="cta-btn" type="button" onClick={onTranscribe}>
              Transcribe
            </button>
          </div>
        </div>
      );
  }
}
