#!/usr/bin/env bash
#
# Remeet setup — one command to get from a fresh clone to a working app:
#   • checks build prerequisites (Rust, bun, cmake)
#   • picks and installs a transcription engine + model
#   • detects your AI CLI (Claude Code / Codex) and wires it up
#   • writes the app's settings
#   • optionally builds and installs Remeet to /Applications
#
# Run from the repo root:  ./setup.sh
#
set -euo pipefail

# ---------- pretty output ----------
if [[ -t 1 ]]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'
  RED=$'\033[31m'; CYAN=$'\033[36m'; RESET=$'\033[0m'
else
  BOLD=""; DIM=""; GREEN=""; YELLOW=""; RED=""; CYAN=""; RESET=""
fi
say()  { printf '%s\n' "$*"; }
step() { printf '\n%s==>%s %s%s\n' "$CYAN$BOLD" "$RESET$BOLD" "$*" "$RESET"; }
ok()   { printf '%s  ✓%s %s\n' "$GREEN" "$RESET" "$*"; }
warn() { printf '%s  !%s %s\n' "$YELLOW" "$RESET" "$*"; }
die()  { printf '%s  ✗ %s%s\n' "$RED" "$*" "$RESET" >&2; exit 1; }

# ask "prompt" "default" -> echoes the answer (prompt shows on the terminal)
ask() {
  local prompt="$1" default="${2:-}" reply
  if [[ -n "$default" ]]; then
    read -rp "$prompt [$default]: " reply || true
    printf '%s' "${reply:-$default}"
  else
    read -rp "$prompt: " reply || true
    printf '%s' "$reply"
  fi
}
# confirm "prompt" -> returns 0 for yes
confirm() {
  local reply
  read -rp "$1 [y/N]: " reply || true
  [[ "$reply" =~ ^[Yy]$ ]]
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WHISPER_DIR="$HOME/whisper"
MODELS_DIR="$WHISPER_DIR/models"
CONFIG_DIR="$HOME/Library/Application Support/com.nativeproductions.remeet"
SETTINGS="$CONFIG_DIR/settings.json"
GGML_BASE="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
VAD_URL="https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin"

say "${BOLD}Remeet setup${RESET}"
say "${DIM}Local-first meeting capture for macOS.${RESET}"

# ---------- 0. platform ----------
[[ "$(uname -s)" == "Darwin" ]] || die "Remeet is macOS-only."

# ---------- 1. prerequisites ----------
step "Checking build prerequisites"
missing=0
if command -v cargo >/dev/null 2>&1; then
  ok "Rust ($(cargo --version | awk '{print $2}'))"
else
  warn "Rust not found — install:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  missing=1
fi
if command -v bun >/dev/null 2>&1; then
  ok "bun ($(bun --version))"
else
  warn "bun not found — install:  curl -fsSL https://bun.sh/install | bash"
  missing=1
fi
if command -v cmake >/dev/null 2>&1; then
  # whisper.cpp is built with cmake; on Apple Silicon an x86_64 cmake (from an Intel
  # Homebrew under /usr/local) makes the build fail. Flag it early.
  if [[ "$(uname -m)" == "arm64" && "$(command -v cmake)" == /usr/local/* ]]; then
    warn "cmake at $(command -v cmake) looks like Intel Homebrew (x86_64); the whisper.cpp build will fail."
    warn "Put an arm64 cmake ahead of it on PATH — see README, 'cmake must match the CPU architecture'."
    missing=1
  else
    ok "cmake ($(cmake --version | head -1 | awk '{print $3}'))"
  fi
else
  warn "cmake not found — needed to build whisper.cpp. Install an arm64 cmake (see README)."
  missing=1
fi
[[ "$missing" -eq 0 ]] || die "Install the tools above, then re-run ./setup.sh"

# ---------- 2. transcription engine + model ----------
step "Transcription engine"
have_builtin=0
if ls "$MODELS_DIR"/ggml-*.bin >/dev/null 2>&1; then have_builtin=1; fi
have_cli=0
if command -v whisper >/dev/null 2>&1 || [[ -x "$WHISPER_DIR/.venv/bin/whisper" ]]; then have_cli=1; fi
[[ "$have_builtin" -eq 1 ]] && say "  ${DIM}found built-in GGML models in $MODELS_DIR${RESET}"
[[ "$have_cli" -eq 1 ]]     && say "  ${DIM}found an OpenAI whisper CLI${RESET}"

say "  1) Built-in (whisper.cpp) — offline, GPU, per-speaker labels ${DIM}[recommended]${RESET}"
say "  2) OpenAI whisper CLI     — cleaner on silence, needs Python"
engine_choice="$(ask "Choose engine (1/2)" "1")"

say ""
say "  Models:  large-v3        most accurate      (~2.9 GB)"
say "           large-v3-turbo  fast, great        (~1.5 GB)  ${DIM}[recommended]${RESET}"
say "           medium / small / base / tiny       (smaller, faster, less accurate)"
model="$(ask "Which model" "large-v3-turbo")"

mkdir -p "$MODELS_DIR"
WHISPER_BIN="whisper"

if [[ "$engine_choice" == "2" ]]; then
  ENGINE="whisper-cli"
  step "Installing the OpenAI whisper CLI"
  command -v python3 >/dev/null 2>&1 || die "python3 not found — install Python 3 first."
  if [[ ! -x "$WHISPER_DIR/.venv/bin/whisper" ]]; then
    say "  creating a virtualenv at $WHISPER_DIR/.venv"
    python3 -m venv "$WHISPER_DIR/.venv"
    "$WHISPER_DIR/.venv/bin/pip" install -q -U pip
    say "  installing openai-whisper (a minute or two)…"
    "$WHISPER_DIR/.venv/bin/pip" install -q -U openai-whisper
  fi
  WHISPER_BIN="$WHISPER_DIR/.venv/bin/whisper"
  ok "whisper CLI at $WHISPER_BIN"
  if confirm "Pre-download the '$model' model now? (several GB)"; then
    if "$WHISPER_DIR/.venv/bin/python" -c "import whisper; whisper.load_model('$model')"; then
      ok "model '$model' ready"
    else
      warn "prefetch failed — it will download on the first transcription instead"
    fi
  fi
else
  ENGINE="builtin"
  step "Downloading the built-in model"
  target="$MODELS_DIR/ggml-$model.bin"
  if [[ -f "$target" ]]; then
    ok "already have $target"
  else
    say "  downloading ggml-$model.bin…"
    curl -L --fail --progress-bar -o "$target" "$GGML_BASE/ggml-$model.bin" \
      || die "could not download ggml-$model.bin — check the model name and your connection."
    ok "saved $target"
  fi
  # Silero VAD skips silence for a big speed-up; optional, so failure is non-fatal.
  vad="$MODELS_DIR/ggml-silero-v5.1.2.bin"
  if [[ ! -f "$vad" ]]; then
    say "  fetching the Silero VAD model (optional)…"
    if curl -L --fail --progress-bar -o "$vad" "$VAD_URL"; then
      ok "saved $vad"
    else
      rm -f "$vad"
      warn "VAD download failed — skipped (optional; the app runs fine without it)"
    fi
  fi
fi

# ---------- 3. AI provider ----------
step "AI provider (for meeting summaries)"
have_claude=0; command -v claude >/dev/null 2>&1 && have_claude=1 || true
have_codex=0;  command -v codex  >/dev/null 2>&1 && have_codex=1  || true
if [[ "$have_claude" -eq 1 && "$have_codex" -eq 1 ]]; then
  say "  Found both Claude Code and Codex."
  pc="$(ask "Use which for summaries? (claude/codex)" "claude")"
  [[ "$pc" == "codex" ]] && PROVIDER="codex" || PROVIDER="claude-code"
  ok "Using $PROVIDER."
elif [[ "$have_claude" -eq 1 ]]; then
  PROVIDER="claude-code"; ok "Using Claude Code."
elif [[ "$have_codex" -eq 1 ]]; then
  PROVIDER="codex"; ok "Using Codex."
else
  PROVIDER="claude-code"
  warn "Neither 'claude' nor 'codex' is on PATH."
  warn "Recording and transcription work without one; summaries need Claude Code or Codex."
  warn "Install one later, then pick it in Settings."
fi

# ---------- 4. write settings ----------
step "Writing settings for the installed app"
mkdir -p "$CONFIG_DIR"
python3 - "$SETTINGS" "$PROVIDER" "$ENGINE" "$model" "$WHISPER_BIN" <<'PY'
import json, sys
path, provider, engine, model, whisper_bin = sys.argv[1:6]
try:
    with open(path) as f:
        s = json.load(f)
    if not isinstance(s, dict):
        s = {}
except Exception:
    s = {}
s["provider"] = provider
s["transcribe_engine"] = engine
if engine == "whisper-cli":
    wc = s.get("whisper_cli") or {}
    wc["bin"] = whisper_bin
    wc["model"] = model
    s["whisper_cli"] = wc
else:
    wb = s.get("whisper_builtin") or {}
    wb["model"] = model
    s["whisper_builtin"] = wb
with open(path, "w") as f:
    json.dump(s, f, indent=2)
    f.write("\n")
print("  wrote " + path)
PY
ok "settings saved"
say "  ${DIM}(the dev build, 'bun run app', keeps separate settings under .../com.nativeproductions.remeet/dev)${RESET}"

# ---------- 5. install ----------
step "Install the app"
if confirm "Build and install Remeet to /Applications now?"; then
  "$ROOT/scripts/update-app.sh"
else
  say "  Skipped. Build and install later with:  ./scripts/update-app.sh"
fi

say ""
say "${GREEN}${BOLD}Setup complete.${RESET} Launch Remeet from Spotlight or Launchpad."
say "First launch prompts for ${BOLD}Microphone${RESET}; for system audio, grant ${BOLD}Screen Recording${RESET}"
say "in System Settings → Privacy & Security, then relaunch Remeet."
