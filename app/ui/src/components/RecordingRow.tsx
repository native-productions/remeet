import { useState } from "react";

import { api, errorText, type Recording } from "../lib/api";
import { duration, relativeTime } from "../lib/format";

type Props = {
  recording: Recording;
  selected?: boolean;
  onOpen: (recording: Recording) => void;
  onDeleted: () => void;
};

export function RecordingRow({ recording, selected, onOpen, onDeleted }: Props) {
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const remove = async () => {
    setDeleting(true);
    try {
      await api.deleteRecording(recording.id);
      onDeleted();
    } catch (e) {
      setError(errorText(e));
      setDeleting(false);
    }
  };

  return (
    <li
      className={`row${confirming ? " is-confirming" : ""}${selected ? " is-selected" : ""}`}
      tabIndex={0}
      onClick={() => !confirming && onOpen(recording)}
      onKeyDown={(e) => {
        if (confirming) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen(recording);
        }
      }}
    >
      <div className="row-main">
        <div className="row-title">{duration(recording.duration_secs)}</div>
        <div className="row-sub">{relativeTime(recording.created)}</div>
      </div>

      <span className={`row-tag ${recording.transcribed ? "done" : "pending"}`}>
        {recording.transcribed ? "Transcribed" : "Transcribe"}
      </span>

      <button
        className="row-del"
        type="button"
        aria-label="Delete recording"
        onClick={(e) => {
          // Stops the row's own click, so asking to delete never also opens it.
          e.stopPropagation();
          setConfirming(true);
        }}
      >
        ×
      </button>

      {confirming && (
        // Deleting erases the audio itself, with no trash to recover it from, so
        // the row asks in place first rather than acting on a single stray click.
        <div className="confirm" onClick={(e) => e.stopPropagation()}>
          <span className="confirm-text">
            {error ?? "Delete this recording?"}
          </span>
          <button
            className="confirm-btn"
            type="button"
            disabled={deleting}
            onClick={() => {
              setConfirming(false);
              setError(null);
            }}
          >
            Cancel
          </button>
          <button
            className="confirm-btn danger"
            type="button"
            disabled={deleting}
            onClick={remove}
          >
            Delete
          </button>
        </div>
      )}
    </li>
  );
}
