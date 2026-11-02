const listen = window.__TAURI__?.event?.listen;
const invoke = window.__TAURI__?.core?.invoke;
const timeline = document.querySelector("#timeline");
const emptyState = document.querySelector("#empty-state");
const count = document.querySelector("#transcript-count");
const badge = document.querySelector("#model-badge");
const detail = document.querySelector("#model-detail");
const progressFill = document.querySelector("#progress-fill");
const progressLabel = document.querySelector("#progress-label");
const progressPercent = document.querySelector("#progress-percent");
const progressBar = document.querySelector(".progress-track");
const transcriptKeys = new Set();
const modelActions = document.querySelector("#model-actions");
const modelPrompt = document.querySelector("#model-prompt");
const modelDirectory = document.querySelector("#model-directory");
const downloadButton = document.querySelector("#download-model");
const modelError = document.querySelector("#model-error");
const saveModelSettingsButton = document.querySelector("#save-model-settings");
const modelSettingsStatus = document.querySelector("#model-settings-status");
const hotkeyShortcut = document.querySelector("#hotkey-shortcut");
const saveHotkeyButton = document.querySelector("#save-hotkey");
const hotkeyStatus = document.querySelector("#hotkey-status");
const shortcutHint = document.querySelector("#shortcut-hint");
const dashboard = document.querySelector(".dashboard");
const startTestButton = document.querySelector("#start-test");
const testStatus = document.querySelector("#test-status");
const testResult = document.querySelector("#test-result");
let activeTest = null;

function showState(data = {}) {
  const state = data.state || "asr_checking";
  dashboard.dataset.modelState = state;
  const percentMatch = `${data.message || ""} ${data.detail || ""}`.match(/(\d+)%/);
  const percent = percentMatch ? Number(percentMatch[1]) : state === "ready" ? 100 : 0;
  badge.textContent = ({ asr_checking: "CHECKING", asr_starting: "STARTING", asr_downloading: "DOWNLOADING", asr_loading: "LOADING", ready: "READY", stopped: "READY", asr_error: "ERROR", error: "ERROR", listening: "LISTENING", finalizing: "FINALIZING", focus_changed: "PAUSED" })[state] || state.toUpperCase();
  badge.classList.toggle("is-ready", state === "ready" || state === "stopped");
  const needsSetup = state === "asr_error" || state === "error";
  modelActions.hidden = !needsSetup;
  modelPrompt.textContent = data.message || "The local recognizer needs attention.";
  modelError.textContent = data.detail || "";
  downloadButton.textContent = "Reload model";
  downloadButton.disabled = false;
  detail.textContent = data.detail || data.message || "Preparing the local speech model";
  const indeterminate = state === "asr_downloading" && !percentMatch;
  progressFill.classList.toggle("is-indeterminate", indeterminate);
  progressFill.style.width = `${indeterminate ? 34 : Math.max(0, Math.min(100, percent))}%`;
  progressPercent.textContent = percentMatch || percent === 100 ? `${percent}%` : "—";
  progressLabel.textContent = ({ asr_checking: "Checking model files", asr_starting: "Starting recognizer", asr_downloading: "Downloading English Zipformer", asr_loading: "Loading English Zipformer", listening: "Streaming speech", finalizing: "Finalizing transcript", asr_error: "Recognizer needs attention" })[state] || "Preparing local speech model";
  if (["asr_checking", "asr_starting", "asr_downloading", "asr_loading", "listening", "finalizing"].includes(state)) modelActions.hidden = true;
  progressBar.setAttribute("aria-valuenow", String(percent));
}

async function copyTextToClipboard(value) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }
  const field = document.createElement("textarea");
  field.value = value;
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.append(field);
  field.select();
  const copied = document.execCommand("copy");
  field.remove();
  if (!copied) throw new Error("Clipboard access is unavailable.");
}

function addTranscript(item) {
  const key = `${item.timestamp}:${item.text}`;
  if (transcriptKeys.has(key)) return;
  transcriptKeys.add(key);
  if (emptyState?.isConnected) emptyState.remove();
  const entry = document.createElement("article");
  entry.className = "entry";
  entry.dataset.key = key;
  const time = document.createElement("time");
  time.dateTime = new Date(item.timestamp).toISOString();
  time.textContent = new Date(item.timestamp).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const text = document.createElement("p");
  text.textContent = item.text;
  const content = document.createElement("div");
  content.className = "entry-content";
  const actions = document.createElement("div");
  actions.className = "entry-actions";
  const copyButton = document.createElement("button");
  copyButton.type = "button";
  copyButton.textContent = "Copy";
  copyButton.title = "Copy transcription";
  copyButton.setAttribute("aria-label", "Copy transcription");
  copyButton.addEventListener("click", async () => {
    try {
      await copyTextToClipboard(item.text);
      copyButton.textContent = "Copied";
      window.setTimeout(() => { copyButton.textContent = "Copy"; }, 1200);
    } catch {
      copyButton.textContent = "Failed";
      window.setTimeout(() => { copyButton.textContent = "Copy"; }, 1600);
    }
  });
  const deleteButton = document.createElement("button");
  deleteButton.type = "button";
  deleteButton.className = "entry-delete";
  deleteButton.textContent = "×";
  deleteButton.title = "Delete transcription";
  deleteButton.setAttribute("aria-label", "Delete transcription");
  deleteButton.addEventListener("click", async () => {
    if (!invoke) return;
    deleteButton.disabled = true;
    try {
      await invoke("delete_transcript", { timestamp: Number(item.timestamp), text: item.text });
      entry.remove();
      count.textContent = String(timeline.querySelectorAll(".entry").length);
      if (count.textContent === "0" && emptyState) timeline.append(emptyState);
    } catch (error) {
      deleteButton.disabled = false;
      deleteButton.title = String(error);
    }
  });
  actions.append(copyButton, deleteButton);
  content.append(text, actions);
  entry.append(time, content);
  timeline.prepend(entry);
  count.textContent = String(timeline.querySelectorAll(".entry").length);
  timeline.scrollTop = 0;
}

showState();

async function initializeDashboard() {
  // Await listener registration before requesting the initial snapshot. This
  // closes the startup gap where a fast transcription could be missed.
  if (listen) {
    await listen("dictation-state", ({ payload }) => showState(payload));
    await listen("transcription", ({ payload }) => addTranscript(payload));
    await listen("transcription-history", ({ payload }) => {
      if (Array.isArray(payload)) payload.forEach((item) => addTranscript(item));
    });
  }
  if (!invoke) return;
  invoke("dashboard_state").then(showState).catch((error) => {
    badge.textContent = "APP ERROR";
    detail.textContent = `Could not read app status: ${error}`;
  });
  invoke("dashboard_recognizer_settings").then((settings) => { modelDirectory.value = settings.model_directory; }).catch(() => {});
  invoke("dashboard_hotkey").then((shortcut) => {
    hotkeyShortcut.value = shortcut;
    shortcutHint.textContent = `Start dictating with ${shortcut}`;
  }).catch(() => {});
  invoke("dashboard_history").then((history) => {
    if (Array.isArray(history)) history.forEach((item) => addTranscript(item));
  }).catch(() => {});
}

initializeDashboard();

if (listen) {
  listen("dictation-state", ({ payload }) => {
    if (!activeTest?.recording) return;
    if (["error", "asr_error", "focus_changed"].includes(payload?.state)) {
      activeTest.recording = false;
      startTestButton.disabled = false;
      startTestButton.textContent = "Start test";
      testStatus.textContent = payload.detail || payload.message || "The test could not start.";
    }
  });
  listen("transcription-test-update", ({ payload }) => {
    if (!activeTest) return;
    if (activeTest.sessionId === null) activeTest.sessionId = payload.sessionId;
    if (payload.sessionId !== activeTest.sessionId) return;
    activeTest.updates.push(payload);
    testStatus.textContent = `Captured stream update ${payload.revision} at ${payload.windowEnd.toFixed(1)} s; ${activeTest.updates.length} update${activeTest.updates.length === 1 ? "" : "s"} recorded.`;
  });
  listen("transcription-test-complete", ({ payload }) => {
    if (!activeTest) return;
    if (activeTest.sessionId === null) activeTest.sessionId = payload.sessionId;
    if (payload.sessionId !== activeTest.sessionId) return;
    activeTest.finishedAt = payload.finishedAt;
    activeTest.finalText = payload.finalText;
    activeTest.recording = false;
    activeTest.startedAt = payload.startedAt;
    activeTest.finishedAt = payload.finishedAt;
    renderTestResult();
    startTestButton.textContent = "Start test";
    startTestButton.disabled = false;
    testStatus.textContent = `Test complete. ${activeTest.updates.length} recognition updates saved; copy the report to share it.`;
  });
}

function renderTestResult() {
  if (!activeTest) return;
  testResult.hidden = false;
  testResult.replaceChildren();
  const finalLabel = document.createElement("strong");
  finalLabel.textContent = "Final transcript";
  const finalText = document.createElement("p");
  finalText.textContent = activeTest.finalText || "(No speech recognized)";
  const updates = document.createElement("details");
  const summary = document.createElement("summary");
  summary.textContent = `Recognition updates (${activeTest.updates.length})`;
  const list = document.createElement("ol");
  activeTest.updates.forEach((update) => {
    const row = document.createElement("li");
    row.textContent = `#${update.revision} · ${update.windowEnd.toFixed(1)} s: ${update.text || "(empty)"}`;
    list.append(row);
  });
  const actions = document.createElement("div");
  actions.className = "test-result-actions";
  const copy = document.createElement("button");
  copy.type = "button";
  copy.textContent = "Copy test report";
  copy.addEventListener("click", async () => {
    const report = { preset: document.querySelector("#test-preset").textContent, ...activeTest };
    try { await copyTextToClipboard(JSON.stringify(report, null, 2)); copy.textContent = "Copied"; }
    catch { copy.textContent = "Copy failed"; }
    window.setTimeout(() => { copy.textContent = "Copy test report"; }, 1400);
  });
  actions.append(copy);
  updates.append(summary, list);
  testResult.append(finalLabel, finalText, updates, actions);
}

startTestButton.addEventListener("click", async () => {
  if (!invoke) return;
  startTestButton.disabled = true;
  if (activeTest?.recording) {
    testStatus.textContent = "Finishing the test and saving its final transcript…";
    try { await invoke("toggle_dictation"); activeTest.recording = false; }
    catch (error) { testStatus.textContent = String(error); startTestButton.disabled = false; }
    return;
  }
  activeTest = { sessionId: null, startedAt: Date.now(), preset: document.querySelector("#test-preset").textContent, updates: [], finalText: "", recording: true };
  testResult.hidden = true;
  testResult.replaceChildren();
  testStatus.textContent = "Listening. Read the preset sentence, then select Stop test.";
  try {
    await invoke("start_transcription_test");
    startTestButton.textContent = "Stop test";
    startTestButton.disabled = false;
  } catch (error) {
    activeTest = null;
    testStatus.textContent = String(error);
    startTestButton.disabled = false;
  }
});
downloadButton.addEventListener("click", async () => {
  if (!invoke) return;
  downloadButton.disabled = true;
  modelError.textContent = "Loading the local Zipformer model…";
  try {
    await invoke("save_recognizer_settings", { modelDirectory: modelDirectory.value });
    modelActions.hidden = true;
  } catch (error) {
    modelError.textContent = String(error);
    downloadButton.disabled = false;
  }
});

saveModelSettingsButton.addEventListener("click", async () => {
  if (!invoke) return;
  saveModelSettingsButton.disabled = true;
  modelSettingsStatus.textContent = "Saving model folder and reloading…";
  try {
    await invoke("save_recognizer_settings", { modelDirectory: modelDirectory.value });
    modelSettingsStatus.textContent = "Model folder saved. The recognizer is loading.";
  } catch (error) {
    modelSettingsStatus.textContent = String(error);
  } finally {
    saveModelSettingsButton.disabled = false;
  }
});

saveHotkeyButton.addEventListener("click", async () => {
  if (!invoke) return;
  saveHotkeyButton.disabled = true;
  hotkeyStatus.textContent = "Updating shortcut…";
  try {
    const shortcut = await invoke("save_dashboard_hotkey", { shortcut: hotkeyShortcut.value });
    hotkeyShortcut.value = shortcut;
    shortcutHint.textContent = `Start dictating with ${shortcut}`;
    hotkeyStatus.textContent = "Shortcut saved and active.";
  } catch (error) {
    hotkeyStatus.textContent = String(error);
  } finally {
    saveHotkeyButton.disabled = false;
  }
});
