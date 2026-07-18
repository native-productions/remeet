import { useEffect, useRef, useState } from "react";

import { DEFAULT_SPACE, type Space } from "../lib/api";

type Props = {
  spaces: Space[];
  value: string | null;
  disabled?: boolean;
  onChange: (id: string | null) => void;
};

/**
 * Picks the space the next recording is filed into.
 *
 * A menu rather than a native select: the list is short, the popover is 340pt wide,
 * and the current choice has to read at a glance next to the record button. It is
 * still a standard menu in behaviour, keyboard and dismissal included.
 */
export function SpacePicker({ spaces, value, disabled, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;

    const onPointerDown = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };

    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const current = value ? spaces.find((s) => s.id === value) : null;
  const label = current?.name ?? DEFAULT_SPACE.name;

  const choose = (id: string | null) => {
    onChange(id);
    setOpen(false);
  };

  return (
    <div className="picker" ref={root}>
      <button
        className="picker-btn"
        type="button"
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="picker-label">{label}</span>
        <span className="picker-caret" aria-hidden="true" />
      </button>

      {open && (
        <div className="picker-menu" role="menu">
          <button
            className={`picker-item${value === null ? " is-current" : ""}`}
            type="button"
            role="menuitem"
            onClick={() => choose(null)}
          >
            {DEFAULT_SPACE.name}
          </button>
          {spaces.map((space) => (
            <button
              key={space.id}
              className={`picker-item${value === space.id ? " is-current" : ""}`}
              type="button"
              role="menuitem"
              onClick={() => choose(space.id)}
            >
              {space.name}
            </button>
          ))}
          {spaces.length === 0 && (
            <p className="picker-empty">Create spaces in the Remeet window.</p>
          )}
        </div>
      )}
    </div>
  );
}
