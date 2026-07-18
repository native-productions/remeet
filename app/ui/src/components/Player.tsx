import { duration } from "../lib/format";
import type { Player as PlayerState } from "../lib/useAudioPlayer";

/** Transport for one recording: play/pause, scrub, and a speed cycle. */
export function Player({ player }: { player: PlayerState }) {
  const { playing, loading, error, position, total, speed } = player;

  return (
    <div
      className={`player${playing ? " is-playing" : ""}${loading ? " is-loading" : ""}${
        error ? " is-error" : ""
      }`}
      // Playback failures are otherwise invisible in a webview with no console.
      title={error ?? undefined}
    >
      <button
        className="play"
        type="button"
        aria-label={playing ? "Pause" : "Play"}
        onClick={() => void player.toggle()}
      >
        <span className="play-icon" aria-hidden="true" />
      </button>

      <input
        className="seek"
        type="range"
        min={0}
        max={1000}
        step={1}
        aria-label="Seek"
        value={total > 0 ? Math.round((position / total) * 1000) : 0}
        onChange={(e) => player.seek((Number(e.target.value) / 1000) * total)}
      />

      <span className="ptime">{duration(position)}</span>

      <button
        className="speed"
        type="button"
        aria-label="Playback speed"
        onClick={player.cycleSpeed}
      >
        {speed}x
      </button>
    </div>
  );
}
