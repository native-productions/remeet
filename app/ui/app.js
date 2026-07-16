// Remeet popover frontend. Talks to the Rust commands over Tauri IPC.

const invoke = window.__TAURI__.core.invoke;

const el = {
  app: document.getElementById("app"),
  tabs: document.getElementById("tabs"),
  tabRecord: document.getElementById("tabRecord"),
  tabLibrary: document.getElementById("tabLibrary"),
  record: document.getElementById("record"),
  library: document.getElementById("library"),
  transcript: document.getElementById("transcript"),
  thead: document.getElementById("thead"),
  recBtn: document.getElementById("recBtn"),
  recState: document.getElementById("recState"),
  recTimer: document.getElementById("recTimer"),
  recordings: document.getElementById("recordings"),
  empty: document.getElementById("empty"),
  back: document.getElementById("back"),
  redo: document.getElementById("redo"),
  tTitle: document.getElementById("tTitle"),
  tSub: document.getElementById("tSub"),
  tbody: document.getElementById("tbody"),
};

let recording = false;
let busy = false;
let currentRec = null;

// Formatting ---------------------------------------------------------------

function duration(secs) {
  const s = Math.max(0, Math.round(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function relativeTime(unixSecs) {
  if (!unixSecs) return "";
  const diff = Math.floor(Date.now() / 1000) - unixSecs;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

// Navigation ---------------------------------------------------------------

function showTab(name) {
  el.tabs.hidden = false;
  el.thead.hidden = true;
  el.transcript.hidden = true;
  el.record.hidden = name !== "record";
  el.library.hidden = name !== "library";

  const isRecord = name === "record";
  el.tabRecord.classList.toggle("is-active", isRecord);
  el.tabLibrary.classList.toggle("is-active", !isRecord);
  el.tabRecord.setAttribute("aria-selected", String(isRecord));
  el.tabLibrary.setAttribute("aria-selected", String(!isRecord));

  if (name === "library") refreshList();
}

function showTranscript() {
  el.tabs.hidden = true;
  el.record.hidden = true;
  el.library.hidden = true;
  el.thead.hidden = false;
  el.transcript.hidden = false;
}

// Status + recording -------------------------------------------------------

async function refreshStatus() {
  try {
    const status = await invoke("get_status");
    setRecording(status.recording, status.elapsed_secs);
  } catch (_) {
    // A failed poll is transient; leave the last known state in place.
  }
}

function setRecording(isRecording, elapsedSecs) {
  recording = isRecording;
  el.app.classList.toggle("is-recording", isRecording);
  el.recBtn.setAttribute("aria-label", isRecording ? "Stop recording" : "Start recording");

  if (isRecording) {
    el.recState.textContent = "Recording";
    el.recTimer.hidden = false;
    el.recTimer.textContent = duration(elapsedSecs);
  } else {
    el.recState.textContent = "Ready to record";
    el.recTimer.hidden = true;
  }
}

async function toggleRecording() {
  if (busy) return;
  busy = true;
  el.recBtn.disabled = true;
  try {
    if (recording) {
      await invoke("stop_recording");
      setRecording(false, 0);
      showTab("library");
    } else {
      await invoke("start_recording");
      setRecording(true, 0);
    }
  } catch (e) {
    el.recState.textContent = String(e);
  } finally {
    busy = false;
    el.recBtn.disabled = false;
  }
}

// Library ------------------------------------------------------------------

async function refreshList() {
  let recordings = [];
  try {
    recordings = await invoke("list_recordings");
  } catch (_) {
    recordings = [];
  }

  el.recordings.innerHTML = "";
  el.recordings.hidden = recordings.length === 0;
  el.empty.hidden = recordings.length > 0;

  for (const rec of recordings) {
    el.recordings.appendChild(row(rec));
  }
}

function row(rec) {
  const li = document.createElement("li");
  li.className = "row";
  li.tabIndex = 0;

  const main = document.createElement("div");
  main.className = "row-main";

  const title = document.createElement("div");
  title.className = "row-title";
  title.textContent = duration(rec.duration_secs);

  const sub = document.createElement("div");
  sub.className = "row-sub";
  sub.textContent = relativeTime(rec.created);

  main.append(title, sub);

  const tag = document.createElement("span");
  tag.className = `row-tag ${rec.transcribed ? "done" : "pending"}`;
  tag.textContent = rec.transcribed ? "Transcribed" : "Transcribe";

  li.append(main, tag);

  const open = () => openTranscript(rec);
  li.addEventListener("click", open);
  li.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      open();
    }
  });

  return li;
}

// Transcript ---------------------------------------------------------------

async function openTranscript(rec) {
  currentRec = rec;
  el.tTitle.textContent = duration(rec.duration_secs);
  el.tSub.textContent = relativeTime(rec.created);
  el.tbody.innerHTML = "";
  el.redo.hidden = true;
  showTranscript();

  let lines = null;
  try {
    lines = await invoke("get_transcript", { id: rec.id });
  } catch (_) {
    lines = null;
  }

  if (lines && lines.length) {
    renderLines(lines);
    el.redo.hidden = false;
  } else {
    renderTranscribeCta(rec);
  }
}

// Re-runs transcription over the same audio, replacing the cached transcript.
// Lets a recording pick up transcription changes (e.g. bleed suppression).
async function reTranscribe() {
  if (!currentRec) return;
  el.redo.hidden = true;
  el.tbody.innerHTML = "";
  const cta = document.createElement("div");
  cta.className = "cta";
  el.tbody.appendChild(cta);
  await runTranscribe(currentRec, cta);
  if (currentRec) el.redo.hidden = false;
}

function renderLines(lines) {
  const frag = document.createDocumentFragment();
  for (const line of lines) {
    const row = document.createElement("div");
    row.className = `line ${line.speaker === "me" ? "me" : "them"}`;

    const tag = document.createElement("div");
    tag.className = "line-tag";
    tag.textContent = line.speaker;

    const body = document.createElement("div");
    const text = document.createElement("div");
    text.className = "line-text";
    text.textContent = line.text;
    const time = document.createElement("div");
    time.className = "line-time";
    time.textContent = duration(line.start_secs);
    body.append(text, time);

    row.append(tag, body);
    frag.appendChild(row);
  }
  el.tbody.innerHTML = "";
  el.tbody.appendChild(frag);
}

function renderTranscribeCta(rec) {
  el.tbody.innerHTML = "";
  const cta = document.createElement("div");
  cta.className = "cta";

  const text = document.createElement("p");
  text.className = "cta-text";
  text.textContent = "This recording has not been transcribed yet.";

  const btn = document.createElement("button");
  btn.className = "cta-btn";
  btn.type = "button";
  btn.textContent = "Transcribe";

  cta.append(text, btn);
  el.tbody.appendChild(cta);

  btn.addEventListener("click", () => runTranscribe(rec, cta));
}

async function runTranscribe(rec, cta) {
  cta.innerHTML = "";
  const working = document.createElement("div");
  working.className = "working";
  working.innerHTML = '<span class="spin"></span>Transcribing on this Mac';
  cta.appendChild(working);

  try {
    const lines = await invoke("transcribe", { id: rec.id });
    renderLines(lines);
  } catch (e) {
    cta.innerHTML = "";
    const err = document.createElement("p");
    err.className = "error";
    err.textContent = String(e);
    const retry = document.createElement("button");
    retry.className = "cta-btn";
    retry.type = "button";
    retry.textContent = "Try again";
    retry.addEventListener("click", () => runTranscribe(rec, cta));
    cta.append(err, retry);
  }
}

// Wiring -------------------------------------------------------------------

el.tabRecord.addEventListener("click", () => showTab("record"));
el.tabLibrary.addEventListener("click", () => showTab("library"));
el.recBtn.addEventListener("click", toggleRecording);
el.back.addEventListener("click", () => showTab("library"));
el.redo.addEventListener("click", reTranscribe);

showTab("record");
refreshStatus();
refreshList();
setInterval(refreshStatus, 1000);
