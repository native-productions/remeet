# Remeet — working notes

Local-first meeting capture for macOS: record both sides of a call, transcribe on
device, turn it into work. Rust core, Tauri shell, React frontend.

Product intent is in `PRODUCT.md`, visual system in `DESIGN.md`, build and spike
instructions in `README.md`. This file covers how to work in the repo.

## Toolchain

**Bun is the package manager and script runner. Not npm, not yarn, not pnpm.**
Every frontend command runs through it, and `bun.lock` is the committed lockfile —
never generate `package-lock.json` or `yarn.lock` alongside it.

```sh
cd app/ui
bun install          # dependencies
bun run dev          # vite alone, no native shell
bun run build        # typecheck + production bundle
bun run app          # tauri dev: vite + the native app together
bun run app:build    # bundled .app
```

Note `bun run <script>`, not `bun <script>` — bare `bun app` tries to execute a file
called `app` and never reaches the script.

The `app` scripts `cd ..` first: the Tauri CLI locates a project by finding
`tauri.conf.json` in the current folder or below, and from `app/ui` the `src-tauri`
directory is a sibling, not a child. The CLI then runs `beforeDevCommand` back in
`app/ui` (it resolves the frontend directory from `package.json`), which is why that
hook is a plain `bun run dev`.

Rust is the workspace at the repo root:

```sh
cargo build
cargo test
cargo run --release -p remeet-app   # runs against the built bundle in app/ui/dist
```

`cargo run` on its own (dev profile) expects the Vite dev server on port 1420,
because `devUrl` points there. Use `bun run app` for the normal dev loop.

### Native build dependencies

The `remeet-aec` crate depends on `webrtc-audio-processing` (bundled), whose build
script compiles vendored C++ with **`meson` + `ninja`** — both must be on `PATH` for
every build, including `bun run app`. Install once (e.g. `brew install meson ninja`, or
`pip install meson ninja` and symlink into `/usr/local/bin`). The release bundle also
needs `MACOSX_DEPLOYMENT_TARGET=15.0` (the same macOS 10.15+ floor `std::filesystem` in
ggml/webrtc requires) — `scripts/update-app.sh` sets it and checks for meson/ninja;
`tauri.conf.json` pins `minimumSystemVersion` to match. Dev builds inherit the SDK's
default target, so they only need meson/ninja.

## Layout

```
crates/           Rust core: audio capture, transcription, AI providers, session
app/src-tauri/    Tauri shell: windows, tray, settings, commands over the core
app/ui/           React frontend (Vite + TS), one bundle for both windows
```

## AI providers

`remeet-ai` wraps the local Claude Code and Codex CLIs behind one `Provider` trait.
Anything that needs a language model goes through it — do not shell out to a CLI
from anywhere else. Provider choice, model, and binary path live in
`settings.json` under the app config directory.

Two constraints that shape the design:

- Every invocation re-pays the CLI's own startup context (~47k tokens for Claude
  Code, ~18k for Codex). Batch per meeting; cache results to disk.
- Models are free text, never a hardcoded menu — allowed models depend on the
  account behind the CLI.

## Two windows, one bundle

`app/ui/src/main.tsx` reads the Tauri window label and renders either `PopoverApp`
(menu bar, capture) or `MainApp` (workspace). Shared pieces live in
`src/components` and `src/lib`; window-specific styling is scoped with
`body[data-window="..."]`.

Keep the popover small. If a feature would make it slower to answer "am I
recording?", it belongs in the main window.

**The popover is hidden, never closed.** Its webview lives for as long as the app
does, so a component that fetches on mount fetches exactly once per app launch and
then goes stale forever. Anything shared between the two windows needs a push
(`emit` from Rust, `listen` in the hook) or a pull on focus
(`getCurrentWindow().onFocusChanged`). See `useSpaces` and `useRecordings`.

## Conventions

- **IPC goes through `src/lib/api.ts`.** Every command wrapper and its types are
  declared there once, not re-guessed per call site.
- **Business logic stays in Rust.** The frontend calls commands; it does not own
  state the backend can own, and it will not own SQL when the database lands.
- **Audio is filesystem, never database.** Recordings are directories under
  `~/Remeet/recordings`; the store keeps paths, not blobs.
- **Per-recording state lives with the recording.** Transcript, summary, mixdown,
  and space membership are all files inside the recording's own directory. Anything
  central would have to be reconciled against the disk on every launch; this cannot
  drift. Config-level state (settings, the list of spaces) goes in the app config
  directory as JSON.
- **Transcripts are untrusted input.** Anything piped to an AI CLI keeps tool access
  denied — a transcript can contain text engineered to look like an instruction.
- **Comments explain why, not what.** Match the density already in the file.
- **Verify before claiming.** Build, run, or exercise the change; if something is
  unverified, say which part.
