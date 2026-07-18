import type { Recording } from "../lib/api";
import { RecordingRow } from "./RecordingRow";

type Props = {
  recordings: Recording[];
  selectedId?: string | null;
  emptyTitle: string;
  emptySub: string;
  onOpen: (recording: Recording) => void;
  onChanged: () => void;
};

export function RecordingList({
  recordings,
  selectedId,
  emptyTitle,
  emptySub,
  onOpen,
  onChanged,
}: Props) {
  if (recordings.length === 0) {
    return (
      <div className="empty">
        <p className="empty-title">{emptyTitle}</p>
        <p className="empty-sub">{emptySub}</p>
      </div>
    );
  }

  return (
    <ul className="recordings">
      {recordings.map((rec) => (
        <RecordingRow
          key={rec.id}
          recording={rec}
          selected={rec.id === selectedId}
          onOpen={onOpen}
          onDeleted={onChanged}
        />
      ))}
    </ul>
  );
}
