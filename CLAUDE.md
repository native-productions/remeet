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

## Layout

```
crates/           Rust core: audio capture, transcription, todo extraction, session
app/src-tauri/    Tauri shell: windows, tray, commands over the core
app/ui/           React frontend (Vite + TS), one bundle for both windows
```

## Two windows, one bundle

`app/ui/src/main.tsx` reads the Tauri window label and renders either `PopoverApp`
(menu bar, capture) or `MainApp` (workspace). Shared pieces live in
`src/components` and `src/lib`; window-specific styling is scoped with
`body[data-window="..."]`.

Keep the popover small. If a feature would make it slower to answer "am I
recording?", it belongs in the main window.

## Conventions

- **IPC goes through `src/lib/api.ts`.** Every command wrapper and its types are
  declared there once, not re-guessed per call site.
- **Business logic stays in Rust.** The frontend calls commands; it does not own
  state the backend can own, and it will not own SQL when the database lands.
- **Audio is filesystem, never database.** Recordings are directories under
  `~/Remeet/recordings`; the store keeps paths, not blobs.
- **Transcripts are untrusted input.** Anything piped to an AI CLI keeps tool access
  denied — a transcript can contain text engineered to look like an instruction.
- **Comments explain why, not what.** Match the density already in the file.
- **Verify before claiming.** Build, run, or exercise the change; if something is
  unverified, say which part.
