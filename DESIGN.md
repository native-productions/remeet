# Remeet design system

A quiet, neutral, native macOS menu-bar surface. White, soft, unobtrusive. The
interface should feel like paper in a bright room, not a screen demanding attention.

## Theme

**Light only.** The physical scene: a developer glancing at a menu-bar popover in a
bright daytime workspace, between coding sessions, to start a recording or read back
what was said. Bright ambient light, quick glances, a calm mood. That forces light.

There is no dark variant. A menu-bar popover is a momentary surface; committing fully
to one light treatment is better than a mediocre pair.

## Color

Strategy: **Restrained.** Neutral surfaces carry everything; a single clay accent
marks the primary action and selection; a distinct coral marks the one thing that
must interrupt calm, a live recording.

All values OKLCH. Neutrals are untinted (chroma 0), so the only colour on screen is
the accent and the live state.

**Revised from warm cream.** The neutrals were originally tinted toward hue ~85 to
read as warm paper. At a 340pt popover the tint had no large field to be warm in and
read instead as a yellow cast on white — a dirty screen, not warm paper. The warmth
now lives entirely in the clay accent, against clean neutrals.

```
--bg:            oklch(1.000 0 0);        /* app / popover background, white         */
--surface:       oklch(0.985 0 0);        /* raised: header, the record panel        */
--panel:         oklch(0.965 0 0);        /* recessed: list rows, input wells        */
--panel-hover:   oklch(0.945 0 0);        /* row hover                               */
--border:        oklch(0.912 0 0);        /* hairlines, 1px                          */
--border-strong: oklch(0.865 0 0);        /* focus rings, dividers that must read    */

--text:          oklch(0.320 0 0);        /* primary, near-black (never #000)        */
--text-soft:     oklch(0.505 0 0);        /* secondary labels                        */
--text-muted:    oklch(0.640 0 0);        /* timestamps, meta, placeholders          */

--accent:        oklch(0.585 0.110 45);   /* clay: primary action, selection         */
--accent-hover:  oklch(0.540 0.115 44);
--accent-weak:   oklch(0.585 0.110 45 / 0.12); /* selected-row wash                  */

--live:          oklch(0.605 0.170 25);   /* coral-red: recording only               */
--live-weak:     oklch(0.605 0.170 25 / 0.14);
--positive:      oklch(0.620 0.075 150);  /* sage: "transcribed", used sparingly     */
```

Rules:
- The accent is for the primary action, current selection, and focus. Not decoration.
- Coral `--live` appears only while recording. Nothing else is allowed to pulse or
  demand attention. That exclusivity is what makes the recording state legible.
- Never `#000` for text: the darkest text is 0.320 L. The background is white, but
  nothing sitting on it goes to either rail.

## Typography

System stack, one family, no display faces:

```
font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif;
```

Fixed rem scale, ratio ~1.2. Numbers (durations, timestamps) use
`font-variant-numeric: tabular-nums` so they don't jitter.

```
--text-xs:  0.6875rem;  /* 11px  meta, timestamps           */
--text-sm:  0.8125rem;  /* 13px  body, list titles          */
--text-base:0.9375rem;  /* 15px  the record state line       */
--text-lg:  1.125rem;   /* 18px  section heading (rare)      */
```

Weights: 400 body, 500 labels/titles, 600 the primary state line and button. Contrast
comes from weight and color, not size inflation.

## Space, radius, elevation

- Spacing scale (px): 2, 4, 6, 8, 12, 16, 20, 24. Vary it for rhythm; the record panel
  breathes more than list rows.
- Radius: popover 14, panels/buttons 10, rows 8, pills 999. Soft, consistent.
- Popover width ~320px. It is a menu-bar surface, not a window.

Elevation is a single soft neutral shadow, slightly deeper than a tinted one would
need to be — on a white popover there is no warmth left to carry the edge:

```
--shadow: 0 1px 2px oklch(0.4 0 0 / 0.07),
          0 8px 24px oklch(0.4 0 0 / 0.14);
```

The popover carries this shadow; interior panels use borders and background steps for
depth, not more shadow. No nested cards.

## Motion

150-220ms, ease-out only. Transform and opacity, never layout properties.

```
--ease: cubic-bezier(0.22, 1, 0.36, 1);   /* ease-out-quint */
--fast: 150ms;
--base: 220ms;
```

- Popover appears with a small translateY + fade (8px, --base).
- The live dot pulses opacity/scale on a slow 1.6s loop; it is the only ambient motion.
- Buttons and rows transition background/color on --fast. No bounce, no choreography.

## Components

Every interactive element defines default / hover / focus / active / disabled. Focus is
a 2px `--border-strong` ring (keyboard), offset 2px; never remove focus outlines.

- **Primary button** (record): filled `--accent`, near-white text, radius 10. While
  recording it becomes `--live` with the pulsing dot and reads "Stop".
- **List rows** (recordings): `--panel`, radius 8, hover `--panel-hover`, selected gets
  `--accent-weak` background and `--accent` left-aligned title. No side-stripe borders.
- **Empty state** teaches: when there are no recordings, explain the one action, don't
  say "nothing here".
- **Loading** (transcribing): inline progress text with the pulsing dot, not a spinner
  dropped in the middle of content.

## Bans (in addition to the shared laws)

No gradients, no glassmorphism, no gradient text, no side-stripe borders, no hero
metric, no card grids, no em dashes in UI copy. Nothing pulses except the live dot.
