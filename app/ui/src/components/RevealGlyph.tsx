/**
 * The "show in Finder" glyph: an arrow leaving a box, the platform-neutral sign for
 * "open this somewhere outside the app". Shared so the flat list and the space
 * browser draw the same mark.
 */
export function RevealGlyph() {
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
      <path d="M9 3h4v4" />
      <path d="M13 3 7.5 8.5" />
      <path d="M11 9.5V12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h2.5" />
    </svg>
  );
}
