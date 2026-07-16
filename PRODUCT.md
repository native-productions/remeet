# Remeet

## Product purpose

A macOS menu-bar app that records a meeting from both sides (your voice and the
remote participants), transcribes it locally, and keeps the transcript. It lives in
the menu bar because it is a companion to whatever you are already doing: you start a
recording, go back to the call, and come back to a transcript.

register: product

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

- **One glance, one action.** The default view answers "am I recording?" and offers
  the single next action. Depth (transcripts, past sessions) is one step in, never in
  your face.
- **Local and quiet.** No accounts, no upload, no telemetry. The UI should reinforce
  that calm: nothing blinks for attention except a live recording.
- **The recording is the product.** Transcripts and, later, action items are derived.
  The UI treats the saved recording as the durable artifact.

## Anti-references

- Zoom / Teams meeting chrome: heavy toolbars, red everywhere, corporate density.
- SaaS dashboards: hero metrics, card grids, gradient accents, marketing energy.
- "AI note-taker" landing-page aesthetics bolted onto an app: glassmorphism, neon,
  animated gradients. This is a quiet native tool, not a pitch.
