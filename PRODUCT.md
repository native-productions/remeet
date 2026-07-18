# Remeet

## Product purpose

A macOS app that records a meeting from both sides (your voice and the remote
participants), transcribes it locally, and turns what was said into work you can act
on. Capture lives in the menu bar because it is a companion to whatever you are
already doing: you start a recording, go back to the call, and come back to a
transcript.

register: product

## Two surfaces

Remeet is deliberately split, because capture and organisation want opposite things.

- **Popover (menu bar)** — the capture surface. Start, stop, glance at status, read
  back a recording. Stays small on purpose. One glance, one action.
- **Main window** — the workspace. Where recordings become organised work: projects,
  knowledge, extracted todos, and AI-assisted processing. Opened on demand from the
  tray or the popover; the app has no dock icon until it is.

**This is a revision, and it should be read as one.** The original framing was "a
background utility, not a destination" — true of the popover, and it is why the
popover must not grow. But projects, a knowledge base, and a todo list are a
destination by definition. Rather than smuggle that into a 340pt panel and ruin what
works, the destination gets its own window. If a feature makes the popover slower to
answer "am I recording?", it belongs in the window.

## Users

One primary user to start: a solo developer who takes calls throughout the day and
wants a durable, searchable record without babysitting a recorder or shipping audio
to someone else's cloud. Comfortable with tools like Raycast, Linear, and native mac
apps. Values speed, quiet, and privacy (everything runs on-device).

## Tone

Calm and unobtrusive. The app is a background utility, not a destination. It should
feel like a well-made native mac tool: it appears when summoned, does one thing
clearly, and gets out of the way. Warm rather than clinical, but never cute.

## Strategic principles

- **One glance, one action.** The popover's default view answers "am I recording?"
  and offers the single next action. Depth (transcripts, past sessions) is one step
  in, never in your face. Depth that needs a desk belongs in the main window.
- **Local and quiet.** No accounts, no upload, no telemetry. The UI should reinforce
  that calm: nothing blinks for attention except a live recording.
- **The recording is the product.** Transcripts and, later, action items are derived.
  The UI treats the saved recording as the durable artifact.

## Anti-references

- Zoom / Teams meeting chrome: heavy toolbars, red everywhere, corporate density.
- SaaS dashboards: hero metrics, card grids, gradient accents, marketing energy.
- "AI note-taker" landing-page aesthetics bolted onto an app: glassmorphism, neon,
  animated gradients. This is a quiet native tool, not a pitch.

## Direction

Recording and transcription are proven. What is being built on top, in order:

1. **Main window shell** — React frontend, two windows off one bundle. *(done)*
2. **SQLite + projects** — a real store under Application Support, with recordings
   grouped into projects. Disk stays the source of truth for audio; the database is
   reconciled against it on startup, never trusted blindly over it.
3. **AI providers** — the todo extractor generalised past the hardcoded `claude`
   CLI, so Claude Code and Codex are interchangeable. Transcripts are untrusted
   input: tool access stays denied whichever provider runs.
4. **Knowledge** — undefined on purpose. Not built until "knowledge is used for X"
   fits in one sentence.
