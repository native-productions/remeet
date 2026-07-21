/**
 * The "rename" glyph: a pencil, the platform-neutral sign for editing a label in
 * place. Drawn at the same weight as the reveal mark so the row's buttons read as a
 * set.
 */
export function RenameGlyph() {
  return (
    <svg
      viewBox="0 0 16 16"
      width="13"
      height="13"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M11 2.5 13.5 5 6 12.5 3 13l.5-3z" />
      <path d="M10 3.5 12.5 6" />
    </svg>
  );
}
