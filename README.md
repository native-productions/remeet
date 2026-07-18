# Remeet

Meeting capture, transcription, and action items for macOS.

Status: **spike + app**. Capture, transcription, action-item extraction, and the
record→transcribe orchestration are proven; a menu-bar app wraps the record and
transcribe flow.

## Demo
<img width="1149" height="892" alt="image" src="https://github.com/user-attachments/assets/6e184fb1-330e-41fa-92ab-6ebd05a020c8" />
<img width="467" height="512" alt="image" src="https://github.com/user-attachments/assets/fb242f6c-2cda-4ecb-a131-6f5e979bfffe" />
<img width="465" height="295" alt="image" src="https://github.com/user-attachments/assets/a4e3d0c1-f520-4caa-8b4c-f5a0e3271d68" />
<img width="1152" height="892" alt="image" src="https://github.com/user-attachments/assets/0d780044-1655-4ba4-921d-54e945c575b7" />
<img width="1150" height="893" alt="image" src="https://github.com/user-attachments/assets/fa955ab8-dc7c-4c63-9032-bfb546259fe5" />
<img width="563" height="544" alt="image" src="https://github.com/user-attachments/assets/cf095a9a-0148-4d5f-bf18-b44f4af9ed3d" />





## Layout

```
crates/
  remeet-audio/       Capture + WAV sink. The only crate that touches Objective-C.
  remeet-transcribe/  Whisper transcription: downmix, resample, decode to segments.
  remeet-ai/          Local AI CLIs (Claude Code, Codex) behind one Provider trait.
  remeet-todo/        Action-item extraction via the local Claude CLI.
  remeet-session/     Orchestration: record a meeting to disk, transcribe it back.
app/
  src-tauri/          Tauri shell: tray, popover + main window, commands over remeet-session.
  ui/                 React frontend (Vite + TS, bun). One bundle serves both windows.
spikes/
  dual-capture/       Throwaway: proves both meeting sides record to separate tracks.
  transcribe/         Throwaway: proves the two tracks decode into an attributed
                      "me vs. them" transcript.
  todo/               Throwaway: proves a transcript becomes an attributed todo list.
  session/            Throwaway: proves the record -> transcribe flow end to end.
```

## Pipeline

```
ScreenCaptureKit ─┬─ system audio (48 kHz stereo) ─┐
                  └─ microphone   (48 kHz mono)  ───┤
                                                    ├─ downmix → 16 kHz mono
                                                    ├─ Whisper (Metal GPU)
                                                    ├─ timestamped segments, per track
                                                    │   merged by time → transcript
                                                    └─ Claude CLI → todos, per owner
```

Both tracks share one capture clock, so merging the two transcripts by timestamp
yields a readable, attributed conversation without speaker diarization. That
attribution carries into the todos: because each line is tagged `me` / `them`, the
extractor can resolve "you handle the deploy" onto the right person's list.

## Design

**Two tracks, one stream.** A meeting has two sides: what the machine plays back
(the remote participants) and what the microphone hears (you). ScreenCaptureKit
delivers both from a single `SCStream` as separate output types, so `remeet-audio`
keeps them separate rather than mixing.

This matters for action items. A mixed track gives you

> "I'll handle the deploy, you take the migration"

with no way to tell whose task is whose. Two tracks answer that without any speaker
diarization — not *who* said it by name, but *me vs. them*, which is the part that
decides whose todo list a line lands on.

Using one stream for both is also why the tracks stay aligned: they share a clock,
so presentation timestamps are directly comparable. Capturing the mic through a
second API (`cpal`, say) would introduce a second clock and drift to correct for.

## Building

Requires the Rust toolchain pinned in `rust-toolchain.toml`, plus a **native
`cmake`** (whisper.cpp is built from source — see the caveat below).

```sh
cargo build
cargo test
```

`cidre` binds Objective-C directly, so there is no Swift bridge and no Xcode floor
beyond the macOS 15 SDK.

### cmake must match the CPU architecture

whisper.cpp is compiled by `whisper-rs-sys` via cmake. On Apple Silicon, an **x86_64
cmake** (e.g. from an Intel Homebrew under `/usr/local`, running through Rosetta)
reports `CMAKE_SYSTEM_PROCESSOR: x86_64`, and the build dies with
`clang: error: unsupported argument 'native' to option '-mcpu='` — an Arm compiler
fed an x86 config.

Use an `arm64` cmake ahead of it on `PATH`. Any native build works; one that avoids
touching an existing Intel Homebrew is a pip install into a throwaway venv:

```sh
python3 -m venv ~/.local/share/remeet-tools/venv     # use an arm64 python
~/.local/share/remeet-tools/venv/bin/pip install cmake
ln -sf ~/.local/share/remeet-tools/venv/bin/cmake ~/.local/bin/cmake  # ~/.local/bin on PATH
```

### Why cidre and not the `screencapturekit` crate

The `screencapturekit` crate pulls in `apple-metal`, whose Swift bridge needs the
macOS 26 SDK → Xcode 26 → macOS Tahoe. It does not build on macOS 15. Its own
build script also gates Swift features on the SDK *major* version only, so a 15.1
SDK gets told it has 15.2 APIs and fails to compile.

`cidre` has no Swift bridge, builds in seconds, and exposes the same
ScreenCaptureKit surface including `setCaptureMicrophone:` (macOS 15+).

## Running the spike

```sh
cargo run --release -p dual-capture
```

Records 30 seconds to `recordings/system.wav` and `recordings/microphone.wav`.

It fails loudly on the two things that are invisible from an exit code: a track
that produced no audio, and audio whose duration disagrees with the wall clock
(which means the sample rate or channel count was misread — the WAV would still
play, just at the wrong speed).

Verify the rest by ear:

```sh
afplay recordings/system.wav
afplay recordings/microphone.wav
```

### Permissions

Needs **Screen Recording** (for system audio, even though no video is captured)
and **Microphone**. Both prompt on first run and are attributed to the enclosing
app bundle — for a bare binary that means the terminal you launch it from, not the
binary itself.

### Wear headphones

Verified formats: system audio arrives as 48 kHz stereo, the built-in microphone as
48 kHz mono.

The track split is only as clean as your acoustic isolation. On headphones the
microphone hears only you and the split is exact. On speakers, the microphone also
picks up the remote participants, so the same speech transcribes on both tracks.

`transcribe_recording` suppresses that bleed where it can: the leaked copy is
acoustically degraded, so Whisper scores it with lower confidence than the clean
source, and the merge drops the lower-confidence duplicate (see the `bleed` module in
`crates/remeet-session/src/transcript.rs`). Energy was tried first and rejected: the
leak's level tracks a fixed gain offset, not who is speaking.

This has a hard limit. When speakers are loud enough that the microphone captures the
entire call, both voices land on both tracks. That is the same audio recorded twice,
not bleed, and no heuristic can separate it. Headphones are the fix.

## Running the transcribe spike

Transcribes the two WAVs from the capture spike into one merged transcript. Needs a
GGML Whisper model; the path is the sole argument.

```sh
# Download a model once (~1.5 GB):
curl -L -o models/ggml-large-v3-turbo.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo.bin

cargo run --release -p transcribe -- models/ggml-large-v3-turbo.bin
```

Note the model is **GGML** (`.bin`), not the PyTorch `.pt` that the Python
`openai-whisper` uses — whisper.cpp needs its own format. Language is auto-detected,
so a meeting that switches between languages is fine.

Output is the two tracks interleaved by timestamp and labelled `me` / `them`:

```
[ 0:14.0 -  0:16.2] them: can you ship the deploy this week?
[ 0:16.5 -  0:19.1] me:   I'll take the deploy, you handle the migration
```

On Metal this runs well faster than real time (~5–13× in testing). The same
headphone caveat applies: bleed on the input shows up as the same line appearing on
both tracks.

## Running the todo spike

Extracts action items from a transcript. Reads `[speaker] text` lines on stdin and
prints todos grouped by owner.

```sh
cargo run -p todo < transcript.txt
```

```
Yours:
  - Deploy auth ke staging (due: hari ini)
      from: "tinggal gua deploy ke staging hari ini"

Theirs:
  - Review PR #142 (due: sebelum jumat)
      from: "eh lu bisa gak review PR gua yang #142 sebelum jumat?"
```

### How it talks to Claude

`remeet-todo` shells out to the **locally installed Claude CLI** (`~/.claude`), using
the machine's existing Claude Code login — no API key, no separate subscription. It
runs the CLI headless (`--print --output-format json --json-schema …`), so the model
returns schema-validated structured output rather than prose to scrape.

The extractor resolves pronouns against the speaker: "you do X" is owned by the
*other* side, and a request plus its acceptance collapse to a single todo. The
transcript is passed as clearly delimited data, and the CLI is invoked with the
filesystem and network tools denied — the transcript is untrusted input, so a prompt
injection inside it can at worst skew the todo list, not run a command.

Each call also pays Claude Code's own startup (a few seconds), so this is built for
after-the-meeting extraction, not per-line streaming.

## Running the session spike

Ties capture and transcription together: record a meeting to disk, then transcribe
it into a saved transcript. Record and transcribe are separate steps — the WAV files
are the hand-off — so a recording can also be reopened and transcribed later.

```sh
# record until Enter, then transcribe:
cargo run --release -p session -- models/ggml-large-v3-turbo.bin

# record a fixed number of seconds (for scripted runs):
cargo run --release -p session -- models/ggml-large-v3-turbo.bin 30

# skip capture and transcribe a recording captured earlier:
cargo run --release -p session -- models/ggml-large-v3-turbo.bin --dir recordings/session-…
```

The transcript is written to `<recording-dir>/transcript.txt` next to the audio.

Todo extraction is deliberately *not* part of this flow — recording and transcript
are the deliverables, and wiring them into `remeet-todo` (or anything else) is left
to the caller.

Capture needs the screen awake and Screen Recording permission granted to whatever
process runs the binary (a locked or sleeping display makes ScreenCaptureKit report
"no display available"). The `--dir` path needs neither, since it only reads files.

## Running the app

The app has two surfaces: a menu-bar popover for capture, and a main window for
everything that needs room. Run it from your own terminal (it needs your GUI session
for the tray and windows).

The frontend is React under `app/ui`, built with Vite and **bun**:

```sh
cd app/ui && bun install     # once

bun run app                  # dev: vite + the native app, with hot reload
```

(`bun run app`, not `bun app` — the latter looks for a file named `app`.)

Without the dev server, run the release build, which uses the bundled frontend:

```sh
cd app/ui && bun run build
cargo run --release -p remeet-app
```

A tray icon appears in the menu bar; click it to open the popover, or use its
right-click menu to open the main window. Record starts and stops a session; each
recording lists with its length, and opening one transcribes it on demand and plays it
back at 1x, 1.5x, or 2x. Recordings are stored under `~/Remeet/recordings`, and the
app expects the Whisper model at `~/whisper/models/ggml-large-v3-turbo.bin`.

The app idles as a macOS `Accessory` — menu bar only, no dock icon — and becomes a
regular app while the main window is open, so it can be cmd-tabbed back to.

Deleting a recording from the library removes its whole directory under
`~/Remeet/recordings` — both track WAVs, the mixdown, and the transcript. There is no
trash and no undo, so the row asks for confirmation in place first.

### Spaces

Recordings are filed into spaces: pick one in the popover before recording, browse
them as folders in the main window. "Default Space" is where anything unfiled lands,
including every recording made before spaces existed.

Membership is stored **inside each recording's directory**, as `meta.json`, not in a
central index. The disk is the source of truth for audio: a recording can be moved,
copied, or deleted in Finder, and the app has to agree with what it finds. A central
index would need reconciling on every launch, and its failure mode is a list claiming
recordings that are not there. Keeping the filing beside the audio makes that
impossible, and deleting a space cannot touch a recording.

The list of spaces itself is `spaces.json` in the app config directory. No database:
filing a few dozen recordings into a few spaces is a directory walk, not a query. That
changes when there is something a walk cannot answer, such as full-text search across
transcripts.

Folder names shown in a space (`Recording - 18 Jul 2026, 14:22`) are labels. The
directory keeps its stable `session-<unix>` id, because that id is already referenced
by saved transcripts, summaries, and settings.

### AI providers

Summaries (and, later, action items) run through an AI CLI that is already installed
and logged in on the machine — **Claude Code** or **Codex**, picked in Settings. No
API key and no second subscription; audio never reaches them, only transcript text
the user asked to process.

Both are driven with a JSON schema, so the answer comes back validated rather than
scraped out of prose. Almost everything else differs, and `remeet-ai` absorbs it:

|             | Claude Code                   | Codex                     |
|-------------|-------------------------------|---------------------------|
| Invocation  | `claude --print`              | `codex exec`              |
| Schema      | inline `--json-schema`        | a file, `--output-schema` |
| Result      | `structured_output` on stdout | a file, `-o`              |
| Tool limits | `--disallowedTools`           | `--sandbox read-only`     |

Two things worth knowing before building on this:

- **Every call re-pays the CLI's startup context** — measured at ~47k input tokens
  for Claude Code and ~18k for Codex on a two-token prompt. Process once per meeting,
  never per line. Summaries are cached to `summary.json` next to the audio for the
  same reason.
- **Codex is less contained than Claude Code.** There is no flag to deny its tools,
  so the containment is `--sandbox read-only` plus an empty working directory and
  `--ephemeral`. Read-only still means readable. Transcripts are untrusted input, so
  the difference is real; see the module docs in `crates/remeet-ai/src/codex.rs`.

Model names are free text, not a menu: which models an account may use is decided by
the CLI and the account behind it. (`gpt-5.1-codex`, for instance, is rejected on a
ChatGPT-account Codex login.) Settings has a Test button that spends a few tokens on
a real round trip — the only honest check that a CLI is logged in and the model is
allowed.

Playback needs both sides on one timeline, so the tracks are summed into a 16 kHz mono
`mixdown.wav` cached next to them on first play — telephone quality, which is all
speech needs, and it reuses the band-limited resampler transcription already runs.

Design context is in `PRODUCT.md` and `DESIGN.md`; the UI is a light-committed neutral
system (OKLCH tokens, white surfaces, clay accent, a coral live state that is the only
thing that pulses). It is a menu-bar utility, so it runs with no dock icon.
