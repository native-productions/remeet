# Contributing to Remeet

Thanks for your interest in Remeet. It is a local-first meeting recorder for macOS —
Rust core, Tauri shell, React frontend — and contributions of all sizes are welcome:
bug fixes, new capabilities, performance work, and documentation.

This guide covers how to get set up, how the repo is organized, the conventions the code
follows, and how to land a change.

## Getting set up

You need the build tools from the [README's Install section](README.md#install): the
Rust toolchain, [bun](https://bun.sh), and a native **arm64** `cmake` (whisper.cpp is
built from source; an x86_64 cmake under Rosetta will fail — see the README caveat).
Remeet targets **macOS 15+ on Apple Silicon**.

```sh
git clone https://github.com/<you>/remeet.git
cd remeet
./setup.sh          # optional: installs a model + wires a provider for a working app
```

You do not need `setup.sh` to develop — it is for getting a *usable* app. To hack on the
code you mainly need a Whisper model on disk (see the README) and the dev loop below.

## The dev loop

**Bun is the package manager and script runner — not npm, not yarn, not pnpm.** The
committed lockfile is `bun.lock`; never generate `package-lock.json` or `yarn.lock`.

```sh
cd app/ui
bun install          # dependencies, once
bun run app          # the whole app: Vite + the native shell, with hot reload
bun run build        # typecheck + production frontend bundle
```

Note `bun run app`, not `bun app` (the latter looks for a file named `app`). The `app`
script `cd ..` first so the Tauri CLI can find `tauri.conf.json`.

The Rust workspace lives at the repo root:

```sh
cargo build
cargo test
cargo check -p remeet-app
```

**The dev build is isolated from your real data.** `bun run app` runs a debug build,
which Remeet detects with `tauri::is_dev()`: it stores recordings under `~/Remeet-dev`
and keeps its settings in a separate `dev` config directory, and it wears a **DEV** badge
in the UI and tray. So you can record, transcribe, and break things in dev without
touching the recordings the installed app owns under `~/Remeet`. The two can even run at
the same time.

To build and install the real app (release) to `/Applications`:

```sh
./scripts/update-app.sh
```

## Repository layout

```
crates/           Rust core
  remeet-audio/       capture: ScreenCaptureKit dual-track (mic + system), via cidre
  remeet-transcribe/  whisper.cpp transcription, bleed isolation, denoise
  remeet-session/     recording model, mixdown, transcript assembly
  remeet-ai/          Claude Code / Codex providers behind one trait
app/src-tauri/    Tauri shell: windows, tray, settings, commands over the core
app/ui/           React frontend (Vite + TS), one bundle for both windows
scripts/          update-app.sh (build + install), bump-version.sh
setup.sh          first-run installer
```

Two windows, one bundle: `app/ui/src/main.tsx` reads the Tauri window label and renders
either the menu-bar `PopoverApp` or the `MainApp` workspace. The popover is **hidden, not
closed** — its webview lives for the whole app lifetime, so anything shared between
windows needs a push (`emit`/`listen`) or a pull on focus, not a fetch-on-mount.

## Conventions

These are load-bearing; please follow them so the codebase stays coherent.

- **IPC goes through `app/ui/src/lib/api.ts`.** Every command wrapper and its types are
  declared there once, not re-guessed per call site.
- **Business logic stays in Rust.** The frontend calls commands; it does not own state
  the backend can own.
- **Audio is filesystem, never database.** Recordings are directories under
  `~/Remeet/recordings`; the store keeps paths, not blobs.
- **Per-recording state lives with the recording.** Transcript, summary, mixdown, name,
  and space membership are files inside the recording's own directory, so nothing central
  has to be reconciled against the disk. Config-level state (settings, the list of spaces)
  is JSON in the app config directory.
- **AI goes through `remeet-ai`.** Anything that needs a language model uses the
  `Provider` trait — do not shell out to a CLI elsewhere. Batch per meeting and cache to
  disk: every invocation re-pays the CLI's startup context. Model names are free text,
  never a hardcoded menu.
- **Transcripts are untrusted input.** Anything piped to an AI CLI keeps tool access
  denied — a transcript can contain text engineered to look like an instruction.
- **Comments explain _why_, not _what_,** and match the density already in the file.
- **Verify before claiming.** Build, run, or exercise a change; if part of it is
  unverified, say which part.

There is no ESLint/Prettier gate; match the style of the surrounding code. Rust is
standard `cargo fmt` / `clippy`-clean.

## Testing

- Rust: `cargo test` (unit tests live beside the code they cover).
- Frontend: `bun run build` runs `tsc --noEmit` — a green typecheck is the bar.
- For anything with runtime behavior, exercise it in `bun run app` and say what you saw.

## Versioning

`app/src-tauri/tauri.conf.json` is the single source of truth for the version (it drives
`package_info().version` and the macOS bundle version); the crate's `Cargo.toml` is kept
in sync. Bump both with:

```sh
./scripts/bump-version.sh 0.2.0
```

The current version shows in Settings, the tray menu, and the tray tooltip.

## Opening a pull request

1. Branch off `main` (`git switch -c fix/short-description`). Don't commit to `main`.
2. Keep commits focused and the message imperative (`fix: drop stale selection after
   transcribe`), explaining the *why* in the body when it isn't obvious.
3. Make sure `cargo test` and `bun run build` pass, and exercise the change in the app.
4. Open the PR against `main` with a short description of the problem and the approach.
   Screenshots or a short clip help for anything visual.

Small, reviewable PRs land faster than large ones. If you are planning a big change,
opening an issue first to align on the approach is welcome.

## Reporting bugs and ideas

Open a GitHub issue. For a bug, include your macOS version, whether you are on the
installed app or a dev build (`Remeet vX.Y.Z` vs `· dev` in the tray menu), and the steps
to reproduce. For a feature, a sentence on the problem it solves is worth more than a
detailed spec.
