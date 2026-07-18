import type { Summary } from "../lib/api";

type State =
  | { kind: "loading" }
  | { kind: "absent" }
  | { kind: "working" }
  | { kind: "ready"; summary: Summary }
  | { kind: "failed"; message: string };

type Props = {
  state: State;
  /** False until the recording has a transcript to summarise. */
  canSummarize: boolean;
  providerLabel: string;
  onSummarize: () => void;
};

export function SummaryBody({ state, canSummarize, providerLabel, onSummarize }: Props) {
  if (state.kind === "loading") return <div className="tbody" />;

  if (state.kind === "working") {
    return (
      <div className="tbody">
        <div className="cta">
          <div className="working">
            <span className="spin" />
            Summarising with {providerLabel}
          </div>
          <p className="cta-text">
            The CLI reloads its own context on every call, so this takes a few
            seconds.
          </p>
        </div>
      </div>
    );
  }

  if (state.kind === "failed") {
    return (
      <div className="tbody">
        <div className="cta">
          <p className="error">{state.message}</p>
          <button className="cta-btn" type="button" onClick={onSummarize}>
            Try again
          </button>
        </div>
      </div>
    );
  }

  if (state.kind === "absent") {
    return (
      <div className="tbody">
        <div className="cta">
          <p className="cta-text">
            {canSummarize
              ? `No summary yet. ${providerLabel} will read the transcript on this Mac.`
              : "Transcribe this recording first — the summary is written from the transcript, not the audio."}
          </p>
          <button
            className="cta-btn"
            type="button"
            disabled={!canSummarize}
            onClick={onSummarize}
          >
            Summarise
          </button>
        </div>
      </div>
    );
  }

  const { summary } = state;
  return (
    <div className="tbody summary">
      <p className="sum-overview">{summary.overview}</p>

      {summary.key_points.length > 0 && (
        <section className="sum-block">
          <h2 className="sum-head">Key points</h2>
          <ul className="sum-list">
            {summary.key_points.map((point, i) => (
              <li key={i}>{point}</li>
            ))}
          </ul>
        </section>
      )}

      {summary.decisions.length > 0 && (
        <section className="sum-block">
          <h2 className="sum-head">Decisions</h2>
          <ul className="sum-list">
            {summary.decisions.map((decision, i) => (
              <li key={i}>{decision}</li>
            ))}
          </ul>
        </section>
      )}

      <button className="sum-redo" type="button" onClick={onSummarize}>
        Re-summarise
      </button>
    </div>
  );
}
