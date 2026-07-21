import { useEffect, useState } from "react";

import {
  api,
  errorText,
  MODEL_OTHER,
  PROVIDERS,
  providerCategory,
  providerKey,
  type ProviderCategory,
  type ProviderConfig,
  type Probe,
  type ProviderId,
  type Settings,
} from "../lib/api";
import { useAppInfo } from "../lib/useAppInfo";

type TestState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok"; reply: string; ms: number }
  | { kind: "failed"; message: string };

/**
 * Settings for the local AI CLIs.
 *
 * Two checks, deliberately separate: a probe (does the binary run?) is free and runs
 * on load, while a test (does a real request come back?) costs tokens and only runs
 * when asked. Conflating them would either lie about being ready or spend money on
 * every visit.
 */
export function SettingsPane() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [probes, setProbes] = useState<Partial<Record<ProviderId, Probe>>>({});
  const [test, setTest] = useState<TestState>({ kind: "idle" });
  const [path, setPath] = useState("");
  // "Other…" picked in the model dropdown: reveals the free-text input for a model
  // not in the curated list. Reset on provider change so it never carries over.
  const [modelIsOther, setModelIsOther] = useState(false);
  const appInfo = useAppInfo();

  useEffect(() => {
    void (async () => {
      try {
        setSettings(await api.getSettings());
        setPath(await api.settingsPath());
      } catch {
        // Settings fall back to defaults on the Rust side, so a read failure here
        // means the window is unusable anyway; nothing useful to show.
      }
      for (const p of PROVIDERS) {
        try {
          const probe = await api.probeProvider(p.id);
          setProbes((current) => ({ ...current, [p.id]: probe }));
        } catch {
          // Leave it unknown rather than claiming it is missing.
        }
      }
    })();
  }, []);

  // Switching provider clears the manual-model flag; the new provider's saved model
  // decides afresh whether the free-text input shows.
  useEffect(() => {
    setModelIsOther(false);
  }, [settings?.provider]);

  if (!settings) return <section className="pane" />;

  // Settings are small and written on every edit: there is no Save button to
  // forget, and no half-applied state between the UI and disk.
  const update = (next: Settings) => {
    setSettings(next);
    setTest({ kind: "idle" });
    void api.saveSettings(next).catch(() => {});
  };

  const active = settings.provider;
  const activeCategory = providerCategory(active);
  const meta = PROVIDERS.find((p) => p.id === active)!;
  const configKey = providerKey(active);
  const config = settings[configKey] as ProviderConfig;
  const setConfig = (patch: Partial<ProviderConfig>) =>
    update({ ...settings, [configKey]: { ...config, ...patch } });

  // A saved model that is not in the curated list also counts as manual entry, so
  // an existing off-list choice shows in the text input rather than vanishing.
  const model = config.model ?? "";
  const modelOffList = model !== "" && !meta.models.includes(model);
  const modelManual = modelIsOther || modelOffList;
  const onModelSelect = (value: string) => {
    if (value === MODEL_OTHER) {
      setModelIsOther(true);
      return;
    }
    setModelIsOther(false);
    setConfig({ model: value || null });
  };

  // Switching family selects that family's first provider, so the model/key fields
  // below always have a concrete provider to edit.
  const setMode = (mode: ProviderCategory) => {
    if (mode === activeCategory) return;
    const first = PROVIDERS.find((p) => p.category === mode);
    if (first) update({ ...settings, provider: first.id });
  };

  const runTest = async () => {
    setTest({ kind: "running" });
    const started = performance.now();
    try {
      const reply = await api.testProvider(active);
      setTest({ kind: "ok", reply, ms: Math.round(performance.now() - started) });
    } catch (e) {
      setTest({ kind: "failed", message: errorText(e) });
    }
  };

  return (
    <section className="pane">
      <header className="col-head">
        <h1 className="col-title">Settings</h1>
      </header>

      <div className="pane-body">
        <section className="field">
          <h2 className="field-head">Meeting reminder</h2>
          <p className="field-hint">
            When another app puts a call on your mic and speakers, Remeet can notify
            you so a meeting never goes unrecorded. Tap the notification to start.
          </p>
          <label className="toggle">
            <input
              type="checkbox"
              checked={settings.call_reminder}
              onChange={(e) =>
                update({ ...settings, call_reminder: e.target.checked })
              }
            />
            <span className="toggle-track" aria-hidden="true">
              <span className="toggle-thumb" />
            </span>
            <span className="toggle-text">Notify me when a call is detected</span>
          </label>
        </section>

        <section className="field">
          <h2 className="field-head">Transcription engine</h2>
          <p className="field-hint">
            Built-in runs offline (whisper.cpp). Whisper CLI shells out to your local
            OpenAI <code>whisper</code> install on the mixdown — cleaner on silence, but
            no per-speaker labels and it must be installed.
          </p>
          <div className="choices">
            {(
              [
                { id: "builtin", label: "Built-in", sub: "Offline, per-speaker" },
                { id: "whisper-cli", label: "Whisper CLI", sub: "External, cleaner" },
              ] as const
            ).map((e) => (
              <label
                key={e.id}
                className={`choice${settings.transcribe_engine === e.id ? " is-active" : ""}`}
              >
                <input
                  type="radio"
                  name="transcribe_engine"
                  checked={settings.transcribe_engine === e.id}
                  onChange={() => {
                    const next = { ...settings, transcribe_engine: e.id };
                    update(next);
                    // On first switch to the CLI, try to find the tool for the user
                    // rather than making them paste a path.
                    if (
                      e.id === "whisper-cli" &&
                      (!settings.whisper_cli.bin ||
                        settings.whisper_cli.bin === "whisper")
                    ) {
                      void api.detectWhisper().then((path) => {
                        if (path)
                          update({
                            ...next,
                            whisper_cli: { ...next.whisper_cli, bin: path },
                          });
                      });
                    }
                  }}
                />
                <span className="choice-main">
                  <span className="choice-label">{e.label}</span>
                  <span className="choice-sub">{e.sub}</span>
                </span>
              </label>
            ))}
          </div>
          {settings.transcribe_engine === "whisper-cli" && (
            <>
              <label className="field-sub">
                <span className="field-sub-label">whisper path</span>
                <input
                  className="input"
                  type="text"
                  spellCheck={false}
                  placeholder="~/whisper-openai/.venv/bin/whisper"
                  value={settings.whisper_cli.bin}
                  onChange={(e) =>
                    update({
                      ...settings,
                      whisper_cli: { ...settings.whisper_cli, bin: e.target.value },
                    })
                  }
                />
              </label>
              <label className="field-sub">
                <span className="field-sub-label">model</span>
                <select
                  className="input"
                  value={settings.whisper_cli.model}
                  onChange={(e) =>
                    update({
                      ...settings,
                      whisper_cli: { ...settings.whisper_cli, model: e.target.value },
                    })
                  }
                >
                  {["turbo", "large-v3", "medium", "small", "base", "tiny"].map((m) => (
                    <option key={m} value={m}>
                      {m}
                    </option>
                  ))}
                </select>
              </label>
            </>
          )}
        </section>

        {settings.transcribe_engine === "builtin" && (
        <section className="field">
          <h2 className="field-head">Transcription</h2>
          <p className="field-hint">
            Accurate uses beam search on the full model — best for Indonesian and
            accented speech, but slow on long meetings. Fast decodes greedily: several
            times quicker, with a real accuracy cost.
          </p>
          <div className="choices">
            {(
              [
                { id: "accurate", label: "Accurate", sub: "Best quality — slower" },
                { id: "fast", label: "Fast", sub: "Several times quicker — rougher" },
              ] as const
            ).map((mode) => (
              <label
                key={mode.id}
                className={`choice${settings.transcribe_speed === mode.id ? " is-active" : ""}`}
              >
                <input
                  type="radio"
                  name="transcribe_speed"
                  checked={settings.transcribe_speed === mode.id}
                  onChange={() =>
                    update({ ...settings, transcribe_speed: mode.id })
                  }
                />
                <span className="choice-main">
                  <span className="choice-label">{mode.label}</span>
                  <span className="choice-sub">{mode.sub}</span>
                </span>
              </label>
            ))}
          </div>

          <label className="field-sub">
            <span className="field-sub-label">model</span>
            <select
              className="input"
              value={settings.whisper_builtin.model}
              onChange={(e) =>
                update({
                  ...settings,
                  whisper_builtin: { ...settings.whisper_builtin, model: e.target.value },
                })
              }
            >
              {["large-v3", "large-v3-turbo", "medium", "small", "base", "tiny"].map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          </label>
          <p className="field-hint">
            Loaded from <code>~/whisper/models/ggml-&lt;model&gt;.bin</code>. Install models
            with <code>setup.sh</code>; large-v3 is the most accurate, turbo the fastest.
          </p>

          <label className="field-sub">
            <span className="field-sub-label">Language</span>
            <select
              className="input"
              value={settings.transcribe_language ?? ""}
              onChange={(e) =>
                update({
                  ...settings,
                  transcribe_language: e.target.value || null,
                })
              }
            >
              <option value="">Auto-detect</option>
              <option value="id">Indonesian</option>
              <option value="en">English</option>
            </select>
          </label>
          <p className="field-hint">
            Auto-detect decides between Indonesian and English from the clearest speech
            in the meeting, shared across both sides. Force a language if a recording
            still comes out wrong.
          </p>

          <label className="toggle" style={{ marginTop: "16px" }}>
            <input
              type="checkbox"
              checked={settings.mic_denoise}
              onChange={(e) =>
                update({ ...settings, mic_denoise: e.target.checked })
              }
            />
            <span className="toggle-track" aria-hidden="true">
              <span className="toggle-thumb" />
            </span>
            <span className="toggle-text">Suppress microphone background noise</span>
          </label>
          <p className="field-hint">
            Strips café clatter and room noise from your side before transcribing. It
            removes noise, not other people talking nearby.
          </p>
        </section>
        )}

        <section className="field">
          <h2 className="field-head">AI provider</h2>
          <p className="field-hint">
            Used for summaries, and for action items as they land. Use a local CLI
            already logged in on this Mac — no key, nothing leaves the machine — or an
            API key for Gemini, OpenAI, or any OpenAI-compatible local model.
          </p>

          <div className="choices">
            {(
              [
                { id: "cli", label: "Local CLI", sub: "Claude Code or Codex — no key" },
                { id: "api", label: "API key", sub: "Gemini, OpenAI, or custom" },
              ] as const
            ).map((m) => (
              <label
                key={m.id}
                className={`choice${activeCategory === m.id ? " is-active" : ""}`}
              >
                <input
                  type="radio"
                  name="provider_mode"
                  checked={activeCategory === m.id}
                  onChange={() => setMode(m.id)}
                />
                <span className="choice-main">
                  <span className="choice-label">{m.label}</span>
                  <span className="choice-sub">{m.sub}</span>
                </span>
              </label>
            ))}
          </div>

          <div className="choices" style={{ marginTop: "12px" }}>
            {PROVIDERS.filter((p) => p.category === activeCategory).map((p) => {
              const probe = probes[p.id];
              const sub =
                probe === undefined
                  ? "checking…"
                  : p.category === "cli"
                    ? probe.installed
                      ? (probe.version ?? "installed")
                      : "not found — install it, or set a path below"
                    : probe.installed
                      ? (probe.version ?? "configured")
                      : (probe.error ?? "not configured");
              return (
                <label
                  key={p.id}
                  className={`choice${active === p.id ? " is-active" : ""}`}
                >
                  <input
                    type="radio"
                    name="provider"
                    checked={active === p.id}
                    onChange={() => update({ ...settings, provider: p.id })}
                  />
                  <span className="choice-main">
                    <span className="choice-label">{p.label}</span>
                    <span className="choice-sub">{sub}</span>
                  </span>
                </label>
              );
            })}
          </div>
        </section>

        <section className="field">
          <h2 className="field-head">Model</h2>
          <p className="field-hint">
            {activeCategory === "cli"
              ? "Pick a model, or Provider default to use whatever the CLI is set to. The list is common choices — pick Other to type an exact name your account allows."
              : "Pick the model to request. The list is common choices — pick Other to type an exact name your key or server allows. Required for API providers."}
          </p>
          <select
            className="input"
            value={modelManual ? MODEL_OTHER : model}
            onChange={(e) => onModelSelect(e.target.value)}
          >
            <option value="">
              {activeCategory === "cli" ? "Provider default" : "Select a model…"}
            </option>
            {meta.models.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
            <option value={MODEL_OTHER}>Other…</option>
          </select>
          {modelManual && (
            <input
              className="input"
              type="text"
              spellCheck={false}
              autoFocus
              style={{ marginTop: "8px" }}
              placeholder={meta.modelHint}
              value={config.model ?? ""}
              onChange={(e) => setConfig({ model: e.target.value.trim() || null })}
            />
          )}
        </section>

        {activeCategory === "cli" && (
          <section className="field">
            <h2 className="field-head">Binary path</h2>
            <p className="field-hint">
              Empty means <code>{PROVIDERS.find((p) => p.id === active)?.bin}</code> on
              your PATH. Set a full path if the app cannot find it.
            </p>
            <input
              className="input"
              type="text"
              spellCheck={false}
              placeholder={`/usr/local/bin/${PROVIDERS.find((p) => p.id === active)?.bin}`}
              value={config.bin ?? ""}
              onChange={(e) => setConfig({ bin: e.target.value.trim() || null })}
            />
          </section>
        )}

        {activeCategory === "api" && (
          <section className="field">
            <h2 className="field-head">API key</h2>
            <p className="field-hint">
              Stored in plain text in this app's settings file.{" "}
              {active === "custom"
                ? "Leave empty if your local server needs no key."
                : "Get one from your provider's console."}
            </p>
            <input
              className="input"
              type="password"
              spellCheck={false}
              autoComplete="off"
              placeholder={active === "custom" ? "optional" : "paste your API key"}
              value={config.api_key ?? ""}
              onChange={(e) => setConfig({ api_key: e.target.value.trim() || null })}
            />
          </section>
        )}

        {active === "custom" && (
          <section className="field">
            <h2 className="field-head">Base URL</h2>
            <p className="field-hint">
              Your OpenAI-compatible endpoint. Ollama:{" "}
              <code>http://localhost:11434/v1</code>. LM Studio:{" "}
              <code>http://localhost:1234/v1</code>.
            </p>
            <input
              className="input"
              type="text"
              spellCheck={false}
              placeholder="http://localhost:11434/v1"
              value={config.base_url ?? ""}
              onChange={(e) => setConfig({ base_url: e.target.value.trim() || null })}
            />
          </section>
        )}

        <section className="field">
          <h2 className="field-head">Test</h2>
          <p className="field-hint">
            Sends one tiny prompt through the provider. This is the only way to know
            it is reachable and the model is allowed — and it does spend tokens.
          </p>
          <div className="test-row">
            <button
              className="cta-btn"
              type="button"
              disabled={test.kind === "running"}
              onClick={() => void runTest()}
            >
              {test.kind === "running" ? "Testing…" : "Run test"}
            </button>
            {test.kind === "ok" && (
              <span className="test-ok">
                Replied “{test.reply}” in {(test.ms / 1000).toFixed(1)}s
              </span>
            )}
          </div>
          {test.kind === "failed" && <pre className="test-error">{test.message}</pre>}
        </section>

        {appInfo && (
          <p className="pane-foot">
            Remeet v{appInfo.version} · {appInfo.dev ? "dev" : "installed"}
          </p>
        )}
        {path && <p className="pane-foot">Stored at {path}</p>}
      </div>
    </section>
  );
}
