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
  onTranscribe: () => void;
};

/** The transcript itself, or whatever stands in for it while there isn't one. */
export function TranscriptBody({ state, onTranscribe }: Props) {
  switch (state.kind) {
    case "loading":
      return <div className="tbody" />;

    case "ready":
      return (
        <div className="tbody">
          {state.lines.map((line, i) => (
            <div
              // Lines have no id of their own; position in the transcript is the
              // only stable identity, and the list is replaced wholesale anyway.
              key={i}
              className={`line ${line.speaker === "me" ? "me" : "them"}`}
            >
              <div className="line-tag">{line.speaker}</div>
              <div>
                <div className="line-text">{line.text}</div>
                <div className="line-time">{duration(line.start_secs)}</div>
              </div>
            </div>
          ))}
        </div>
      );

    case "working":
      return (
        <div className="tbody">
          <div className="cta">
            <div className="working">
              <span className="spin" />
              Transcribing on this Mac
            </div>
          </div>
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
