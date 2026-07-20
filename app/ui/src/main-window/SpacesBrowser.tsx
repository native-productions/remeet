import { useMemo, useState } from "react";

import { api, DEFAULT_SPACE, errorText, type Recording, type Space } from "../lib/api";
import { RevealGlyph } from "../components/RevealGlyph";
import { duration, folderName, relativeTime } from "../lib/format";

export type Opened = { recording: Recording; view: "transcript" | "summary" };

type Props = {
  spaces: Space[];
  recordings: Recording[];
  /** The space currently being browsed; null is the default space. */
  spaceId: string | null;
  /** True when no space has been opened yet, so the list of spaces shows. */
  atRoot: boolean;
  openedId: string | null;
  /** Which of the two views is showing, so only that leaf reads as current. */
  openedView: "transcript" | "summary";
  onEnterSpace: (id: string | null) => void;
  onLeaveSpace: () => void;
  onOpen: (opened: Opened) => void;
  onSpacesChanged: () => void;
  /** Recordings changed on disk (a filed recording was deleted): re-read the list. */
  onRecordingsChanged: () => void;
};

/**
 * Spaces, browsed like folders.
 *
 * Two levels, both in one column: the spaces themselves, then the recordings filed
 * into one. A recording expands in place to reveal what can be read from it, which
 * is how a file browser behaves and avoids a third column for two links.
 */
export function SpacesBrowser({
  spaces,
  recordings,
  spaceId,
  atRoot,
  openedId,
  openedView,
  onEnterSpace,
  onLeaveSpace,
  onOpen,
  onSpacesChanged,
  onRecordingsChanged,
}: Props) {
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  const counts = useMemo(() => {
    const map = new Map<string | null, number>();
    for (const rec of recordings) {
      const key = rec.space && spaces.some((s) => s.id === rec.space) ? rec.space : null;
      map.set(key, (map.get(key) ?? 0) + 1);
    }
    return map;
  }, [recordings, spaces]);

  const create = async () => {
    try {
      await api.createSpace(name, description);
      setName("");
      setDescription("");
      setCreating(false);
      setError(null);
      onSpacesChanged();
    } catch (e) {
      setError(errorText(e));
    }
  };

  if (atRoot) {
    return (
      <>
        <header className="col-head">
          <h1 className="col-title">Spaces</h1>
          <button
            className="ghost-btn"
            type="button"
            onClick={() => setCreating((c) => !c)}
          >
            {creating ? "Cancel" : "New space"}
          </button>
        </header>

        <div className="col-body">
          {creating && (
            // Inline, not a dialog: making a space is one name and one line of
            // context, and interrupting the window for that is theatre.
            <form
              className="new-space"
              onSubmit={(e) => {
                e.preventDefault();
                void create();
              }}
            >
              <input
                className="input"
                type="text"
                autoFocus
                placeholder="Name"
                value={name}
                onChange={(e) => setName(e.target.value)}
              />
              <input
                className="input"
                type="text"
                placeholder="What goes in here (optional)"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
              />
              {error && <p className="new-space-error">{error}</p>}
              <button className="cta-btn" type="submit" disabled={!name.trim()}>
                Create space
              </button>
            </form>
          )}

          <ul className="folders">
            <li>
              <SpaceRow
                name={DEFAULT_SPACE.name}
                description={DEFAULT_SPACE.description}
                count={counts.get(null) ?? 0}
                onOpen={() => onEnterSpace(null)}
              />
            </li>
            {spaces.map((space) => (
              <li key={space.id}>
                <SpaceRow
                  name={space.name}
                  description={space.description}
                  count={counts.get(space.id) ?? 0}
                  onOpen={() => onEnterSpace(space.id)}
                  onDelete={async () => {
                    await api.deleteSpace(space.id);
                    onSpacesChanged();
                  }}
                />
              </li>
            ))}
          </ul>
        </div>
      </>
    );
  }

  const space = spaceId ? spaces.find((s) => s.id === spaceId) : null;
  const title = space?.name ?? DEFAULT_SPACE.name;
  const inSpace = recordings.filter((rec) => {
    const filed = rec.space && spaces.some((s) => s.id === rec.space) ? rec.space : null;
    return filed === spaceId;
  });

  return (
    <>
      <header className="col-head">
        <button className="crumb" type="button" onClick={onLeaveSpace}>
          Spaces
        </button>
        <span className="crumb-sep" aria-hidden="true">
          /
        </span>
        <h1 className="col-title">{title}</h1>
      </header>

      <div className="col-body">
        {inSpace.length === 0 ? (
          <div className="empty">
            <p className="empty-title">Nothing filed here yet</p>
            <p className="empty-sub">
              Pick this space in the menu bar before you start recording.
            </p>
          </div>
        ) : (
          <ul className="folders">
            {inSpace.map((rec) => (
              <li key={rec.id}>
                <SpaceRecordingRow
                  recording={rec}
                  isOpen={expanded === rec.id}
                  openedId={openedId}
                  openedView={openedView}
                  onToggle={() =>
                    setExpanded((cur) => (cur === rec.id ? null : rec.id))
                  }
                  onOpen={onOpen}
                  onDeleted={onRecordingsChanged}
                />
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}

/**
 * One filed recording, browsed as a folder: a disclosure toggle that reveals its
 * transcript and summary leaves, plus an in-place delete.
 *
 * Delete lives here as well as on the flat Recordings list because a space is the
 * other place the same recording is seen — pruning a project without leaving the
 * space it belongs to is the whole point of browsing one.
 */
function SpaceRecordingRow({
  recording,
  isOpen,
  openedId,
  openedView,
  onToggle,
  onOpen,
  onDeleted,
}: {
  recording: Recording;
  isOpen: boolean;
  openedId: string | null;
  openedView: "transcript" | "summary";
  onToggle: () => void;
  onOpen: (opened: Opened) => void;
  onDeleted: () => void;
}) {
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
    <>
      <div
        className={`folder rec-folder${isOpen ? " is-open" : ""}${
          confirming ? " is-confirming" : ""
        }`}
      >
        <button
          className="folder-hit"
          type="button"
          aria-expanded={isOpen}
          onClick={onToggle}
        >
          <span className="folder-caret" aria-hidden="true" />
          <span className="folder-main">
            <span className="folder-name">{folderName(recording.created)}</span>
            <span className="folder-sub">
              {duration(recording.duration_secs)} · {relativeTime(recording.created)}
            </span>
          </span>
        </button>

        {!confirming && (
          <button
            className="row-reveal"
            type="button"
            aria-label="Show in Finder"
            title="Show in Finder"
            onClick={() => void api.revealRecording(recording.id).catch(() => {})}
          >
            <RevealGlyph />
          </button>
        )}

        {!confirming && (
          <button
            className="row-del"
            type="button"
            aria-label="Delete recording"
            onClick={() => setConfirming(true)}
          >
            ×
          </button>
        )}

        {confirming && (
          // Deleting erases the audio itself, with no trash to recover it from, so
          // the row asks in place first — the same guard the flat list uses.
          <div className="confirm">
            <span className="confirm-text">{error ?? "Delete this recording?"}</span>
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
              onClick={() => void remove()}
            >
              Delete
            </button>
          </div>
        )}
      </div>

      {isOpen && (
        <div className="leaves">
          <button
            className={`leaf${
              openedId === recording.id && openedView === "transcript"
                ? " is-current"
                : ""
            }`}
            type="button"
            onClick={() => onOpen({ recording, view: "transcript" })}
          >
            Transcript
            {!recording.transcribed && <span className="leaf-note">not yet</span>}
          </button>
          <button
            className={`leaf${
              openedId === recording.id && openedView === "summary"
                ? " is-current"
                : ""
            }`}
            type="button"
            onClick={() => onOpen({ recording, view: "summary" })}
          >
            Summary
            {!recording.summarized && <span className="leaf-note">not yet</span>}
          </button>
        </div>
      )}
    </>
  );
}

function SpaceRow({
  name,
  description,
  count,
  onOpen,
  onDelete,
}: {
  name: string;
  description: string;
  count: number;
  onOpen: () => void;
  onDelete?: () => Promise<void>;
}) {
  const [confirming, setConfirming] = useState(false);

  return (
    <div className={`folder space-row${confirming ? " is-confirming" : ""}`}>
      <button className="folder-hit" type="button" onClick={onOpen}>
        <span className="folder-main">
          <span className="folder-name">{name}</span>
          <span className="folder-sub">
            {description || `${count} recording${count === 1 ? "" : "s"}`}
          </span>
        </span>
      </button>

      <span className="folder-count">{count}</span>

      {onDelete && !confirming && (
        <button
          className="row-del"
          type="button"
          aria-label={`Delete ${name}`}
          onClick={() => setConfirming(true)}
        >
          ×
        </button>
      )}

      {confirming && onDelete && (
        <div className="confirm">
          {/* Deleting a space is safe by construction, and saying so is what stops
              the user hesitating over whether their audio is at risk. */}
          <span className="confirm-text">Delete space? Recordings are kept.</span>
          <button
            className="confirm-btn"
            type="button"
            onClick={() => setConfirming(false)}
          >
            Cancel
          </button>
          <button
            className="confirm-btn danger"
            type="button"
            onClick={() => void onDelete()}
          >
            Delete
          </button>
        </div>
      )}
    </div>
  );
}
