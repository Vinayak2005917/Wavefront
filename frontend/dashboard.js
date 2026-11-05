const listen = window.__TAURI__?.event?.listen;
const invoke = window.__TAURI__?.core?.invoke;
const timeline = document.querySelector("#timeline");
const emptyState = document.querySelector("#empty-state");
const historyRecordingToggle = document.querySelector("#history-recording");
const deleteHistoryButton = document.querySelector("#delete-history");
const historyStatus = document.querySelector("#history-status");
const activityMonths = document.querySelector("#activity-months");
const activityCalendar = document.querySelector("#activity-calendar");
const activityCalendarDescription = document.querySelector("#activity-calendar-description");
const activityTotal = document.querySelector("#activity-total");
const detail = document.querySelector("#model-detail");
const progressFill = document.querySelector("#progress-fill");
const progressLabel = document.querySelector("#progress-label");
const progressPercent = document.querySelector("#progress-percent");
const progressBar = document.querySelector(".progress-track");
const transcripts = new Map();
const expandedDates = new Set();
let initialDateExpansionSet = false;
let historyRecording = true;
let historyDeletePending = false;
const modelActions = document.querySelector("#model-actions");
const modelPrompt = document.querySelector("#model-prompt");
const modelDirectory = document.querySelector("#model-directory");
const downloadButton = document.querySelector("#download-model");
const modelError = document.querySelector("#model-error");
const pickModelDirectoryButton = document.querySelector("#pick-model-directory");
const modelSettingsStatus = document.querySelector("#model-settings-status");
const editHotkeyButton = document.querySelector("#edit-hotkey");
const hotkeyKeycaps = document.querySelector("#hotkey-keycaps");
const hotkeyFeedback = document.querySelector("#hotkey-feedback");
const shortcutHint = document.querySelector("#shortcut-hint");
const dashboard = document.querySelector(".dashboard");
let savedShortcut = "Ctrl + Q";
let capturingShortcut = false;
let savingShortcut = false;
let shortcutChanged = false;
let savedModelDirectory = "";
let modelDirectoryEdited = false;
let modelSaveTimer = 0;
let modelSaveVersion = 0;
let modelSaveQueue = Promise.resolve();
const modelSavePending = new Set();
const directoryMeasureContext = document.createElement("canvas").getContext("2d");

function resizeModelDirectoryInput() {
  const wrapper = modelDirectory.closest(".directory-input-wrap");
  if (!wrapper || !directoryMeasureContext) return;
  const style = window.getComputedStyle(modelDirectory);
  directoryMeasureContext.font = style.font;
  const textWidth = directoryMeasureContext.measureText(modelDirectory.value || "").width;
  const extraWidth = [style.paddingLeft, style.paddingRight, style.borderLeftWidth, style.borderRightWidth]
    .map((value) => Number.parseFloat(value) || 0)
    .reduce((total, value) => total + value, 0) + 8;
  const width = Math.max(120, Math.ceil(textWidth + extraWidth));
  wrapper.style.width = `${width}px`;
  modelDirectory.title = modelDirectory.value;
}

function saveModelDirectory(path) {
  const directory = path.trim();
  if (!invoke || !directory || (directory === savedModelDirectory && modelSavePending.size === 0) || modelSavePending.has(directory)) return modelSaveQueue;
  modelSavePending.add(directory);
  const requestVersion = ++modelSaveVersion;
  modelSettingsStatus.textContent = "Saving folder and reloading the recognizer…";
  modelSaveQueue = modelSaveQueue.catch(() => {}).then(async () => {
    try {
      await invoke("save_recognizer_settings", { modelDirectory: directory });
      savedModelDirectory = directory;
      if (requestVersion === modelSaveVersion) {
        modelSettingsStatus.textContent = "Folder saved. The recognizer is loading.";
      }
    } catch (error) {
      if (requestVersion === modelSaveVersion) modelSettingsStatus.textContent = String(error);
    } finally {
      modelSavePending.delete(directory);
    }
  });
  return modelSaveQueue;
}

function scheduleModelDirectorySave(immediate = false) {
  window.clearTimeout(modelSaveTimer);
  const directory = modelDirectory.value.trim();
  if (!directory) {
    modelSettingsStatus.textContent = "Choose a folder for the local speech models.";
    return;
  }
  if (directory === savedModelDirectory && modelSavePending.size === 0) {
    modelSettingsStatus.textContent = "";
    return;
  }
  if (modelSavePending.has(directory)) return;
  if (immediate) {
    void saveModelDirectory(directory);
  } else {
    modelSettingsStatus.textContent = "Changes save automatically.";
    modelSaveTimer = window.setTimeout(() => {
      void saveModelDirectory(modelDirectory.value);
    }, 700);
  }
}

function renderShortcutKeycaps(shortcut) {
  const keys = String(shortcut || "Ctrl + Q").split("+").map((key) => key.trim()).filter(Boolean);
  hotkeyKeycaps.replaceChildren();
  keys.forEach((key, index) => {
    if (index > 0) {
      const plus = document.createElement("span");
      plus.className = "shortcut-plus";
      plus.textContent = "+";
      hotkeyKeycaps.append(plus);
    }
    const keycap = document.createElement("kbd");
    keycap.className = "keycap";
    keycap.textContent = key;
    hotkeyKeycaps.append(keycap);
  });
  hotkeyKeycaps.setAttribute("aria-label", `Current dictation shortcut: ${keys.join(" plus ")}`);
  editHotkeyButton.setAttribute("aria-label", `Current shortcut: ${keys.join(" plus ")}. Click to change it.`);
}

function renderShortcutCapture(modifiers, key = "") {
  hotkeyKeycaps.replaceChildren();
  const keys = [...modifiers, ...(key ? [key] : [])];
  if (keys.length === 0) {
    const prompt = document.createElement("span");
    prompt.className = "shortcut-prompt";
    prompt.textContent = "Press a shortcut";
    hotkeyKeycaps.append(prompt);
  } else {
    keys.forEach((value, index) => {
      if (index > 0) {
        const plus = document.createElement("span");
        plus.className = "shortcut-plus";
        plus.textContent = "+";
        hotkeyKeycaps.append(plus);
      }
      const keycap = document.createElement("kbd");
      keycap.className = "keycap";
      keycap.textContent = value;
      hotkeyKeycaps.append(keycap);
    });
    if (!key) {
      const plus = document.createElement("span");
      plus.className = "shortcut-plus";
      plus.textContent = "+";
      const placeholder = document.createElement("span");
      placeholder.className = "shortcut-prompt";
      placeholder.textContent = "…";
      hotkeyKeycaps.append(plus, placeholder);
    }
  }
  hotkeyKeycaps.setAttribute("aria-label", keys.length ? keys.join(" plus ") : "Press a shortcut");
}

function shortcutModifiers(event) {
  return [
    ...(event.metaKey ? ["Win"] : []),
    ...(event.ctrlKey ? ["Ctrl"] : []),
    ...(event.altKey ? ["Alt"] : []),
    ...(event.shiftKey ? ["Shift"] : []),
  ];
}

function shortcutKeyName(event) {
  const key = event.key;
  if (/^[a-z0-9]$/i.test(key)) return key.toUpperCase();
  if (/^F(?:[1-9]|1[0-9]|2[0-4])$/.test(key)) return key.toUpperCase();
  return ({
    " ": "Space", Tab: "Tab", Enter: "Enter", Escape: "Escape",
    Backspace: "Backspace", Delete: "Delete", Insert: "Insert",
    Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown",
    ArrowLeft: "Left", ArrowUp: "Up", ArrowRight: "Right", ArrowDown: "Down",
  })[key] || "";
}

function finishShortcutCapture() {
  capturingShortcut = false;
  savingShortcut = false;
  editHotkeyButton.classList.remove("is-recording");
  editHotkeyButton.setAttribute("aria-pressed", "false");
  editHotkeyButton.title = "Click to change the shortcut";
}

async function beginShortcutCapture() {
  if (!invoke) {
    hotkeyFeedback.textContent = "Shortcut editing is unavailable.";
    return;
  }
  capturingShortcut = true;
  savingShortcut = true;
  hotkeyFeedback.textContent = "Pausing shortcut…";
  editHotkeyButton.classList.add("is-recording");
  editHotkeyButton.setAttribute("aria-pressed", "true");
  editHotkeyButton.setAttribute("aria-label", "Pausing the current shortcut before recording a new one.");
  editHotkeyButton.title = "Pausing the current shortcut…";
  try {
    await invoke("suspend_dashboard_hotkey");
    savingShortcut = false;
    hotkeyFeedback.textContent = "Esc to cancel";
    editHotkeyButton.setAttribute("aria-label", "Recording shortcut. Press a modifier and one key. Escape to cancel.");
    editHotkeyButton.title = "Press a modifier and one key. Escape to cancel.";
    renderShortcutCapture([]);
  } catch (error) {
    finishShortcutCapture();
    renderShortcutKeycaps(savedShortcut);
    hotkeyFeedback.textContent = String(error);
    editHotkeyButton.title = String(error);
  }
}

async function cancelShortcutCapture() {
  if (!capturingShortcut || savingShortcut) return;
  capturingShortcut = false;
  savingShortcut = true;
  editHotkeyButton.classList.remove("is-recording");
  editHotkeyButton.setAttribute("aria-pressed", "false");
  editHotkeyButton.title = "Click to change the shortcut";
  renderShortcutKeycaps(savedShortcut);
  hotkeyFeedback.textContent = "Restoring shortcut…";
  try {
    await invoke("resume_dashboard_hotkey");
    hotkeyFeedback.textContent = "";
  } catch (error) {
    hotkeyFeedback.textContent = String(error);
    editHotkeyButton.title = String(error);
  } finally {
    savingShortcut = false;
  }
}

async function saveCapturedShortcut(shortcut) {
  if (!invoke) {
    hotkeyFeedback.textContent = "Shortcut saving is unavailable.";
    return;
  }
  savingShortcut = true;
  hotkeyFeedback.textContent = "Saving…";
  try {
    const saved = await invoke("save_dashboard_hotkey", { shortcut });
    savedShortcut = saved;
    shortcutChanged = true;
    renderShortcutKeycaps(saved);
    shortcutHint.textContent = `Start dictating with ${saved}`;
    finishShortcutCapture();
    hotkeyFeedback.textContent = "Saved";
    window.setTimeout(() => {
      if (!capturingShortcut) hotkeyFeedback.textContent = "";
    }, 1200);
  } catch (error) {
    savingShortcut = false;
    hotkeyFeedback.textContent = String(error);
    editHotkeyButton.title = String(error);
  }
}

function showState(data = {}) {
  const state = data.state || "asr_checking";
  dashboard.dataset.modelState = state;
  const percentMatch = `${data.message || ""} ${data.detail || ""}`.match(/(\d+)%/);
  const percent = percentMatch ? Number(percentMatch[1]) : state === "ready" ? 100 : 0;
  const needsSetup = state === "asr_error" || state === "error";
  modelActions.hidden = !needsSetup;
  modelPrompt.textContent = data.message || "The local recognizer needs attention.";
  modelError.textContent = data.detail || "";
  downloadButton.textContent = "Reload model";
  downloadButton.disabled = false;
  const statusText = {
    asr_checking: "Checking CPU runtime",
    asr_starting: "Starting recognizer",
    asr_downloading: percentMatch ? `Downloading model (${percentMatch[1]}%)` : "Downloading model",
    asr_loading: "Loading speech model",
    ready: "Ready",
    stopped: "Ready",
    listening: "Listening…",
    asr_error: data.message || "Recognizer needs attention",
    error: data.message || "Attention needed",
    focus_changed: data.message || "Text insertion paused",
  }[state] || state;
  if (state !== "finalizing") {
    detail.textContent = statusText;
    detail.title = data.detail || data.message || "";
  }
  const indeterminate = state === "asr_downloading" && !percentMatch;
  progressFill.classList.toggle("is-indeterminate", indeterminate);
  progressFill.style.width = `${indeterminate ? 34 : Math.max(0, Math.min(100, percent))}%`;
  progressPercent.textContent = percentMatch || percent === 100 ? `${percent}%` : "—";
  progressLabel.textContent = ({ asr_checking: "Preparing Nemotron 3.5", asr_starting: "Starting recognizer", asr_downloading: "Downloading speech model", asr_loading: "Loading speech model", listening: "Streaming speech", asr_error: "Recognizer needs attention" })[state] || "Preparing local speech model";
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

function transcriptDate(item) {
  const date = new Date(Number(item.timestamp));
  return Number.isNaN(date.getTime()) ? null : date;
}

function dateKey(date) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

function addTranscript(item, expandNewDate = false) {
  if (!item || typeof item.text !== "string") return;
  const date = transcriptDate(item);
  if (!date) return;
  const key = `${item.timestamp}:${item.text}`;
  if (transcripts.has(key)) return;
  const day = dateKey(date);
  const dayAlreadyExists = [...transcripts.values()].some((saved) => {
    const savedDate = transcriptDate(saved);
    return savedDate && dateKey(savedDate) === day;
  });
  transcripts.set(key, item);
  if (expandNewDate && !dayAlreadyExists) expandedDates.add(day);
  renderHistory(expandNewDate);
}

function updateHistoryControls() {
  deleteHistoryButton.disabled = !invoke || transcripts.size === 0 || historyDeletePending;
}

function makeTranscriptEntry(item, key) {
  const date = transcriptDate(item);
  const entry = document.createElement("article");
  entry.className = "entry";
  entry.dataset.key = key;
  const time = document.createElement("time");
  time.dateTime = date.toISOString();
  time.textContent = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
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
      transcripts.delete(key);
      renderHistory();
    } catch (error) {
      deleteButton.disabled = false;
      deleteButton.title = String(error);
    }
  });
  actions.append(copyButton, deleteButton);
  content.append(text, actions);
  entry.append(time, content);
  return entry;
}

function renderHistory(scrollToTop = false) {
  const scrollTop = timeline.scrollTop;
  const groups = new Map();
  for (const [key, item] of transcripts) {
    const date = transcriptDate(item);
    if (!date) continue;
    const day = dateKey(date);
    if (!groups.has(day)) groups.set(day, { date, items: [] });
    groups.get(day).items.push({ key, item, date });
  }
  const sortedDays = [...groups.keys()].sort((a, b) => b.localeCompare(a));
  if (!sortedDays.length) {
    expandedDates.clear();
    initialDateExpansionSet = false;
    timeline.replaceChildren(emptyState);
    updateHistoryControls();
    renderActivityCalendar();
    return;
  }
  if (!initialDateExpansionSet) {
    expandedDates.add(sortedDays[0]);
    initialDateExpansionSet = true;
  }

  const fragment = document.createDocumentFragment();
  const dateFormatter = new Intl.DateTimeFormat(undefined, {
    weekday: "long", year: "numeric", month: "long", day: "numeric",
  });
  for (const day of sortedDays) {
    const group = groups.get(day);
    group.items.sort((a, b) => Number(b.item.timestamp) - Number(a.item.timestamp));
    const isOpen = expandedDates.has(day);
    const dateGroup = document.createElement("section");
    dateGroup.className = "date-group";
    dateGroup.classList.toggle("is-open", isOpen);
    dateGroup.dataset.date = day;
    const heading = document.createElement("button");
    heading.type = "button";
    heading.className = "date-heading";
    heading.setAttribute("aria-expanded", String(isOpen));
    const entriesId = `date-entries-${day}`;
    heading.setAttribute("aria-controls", entriesId);
    heading.addEventListener("click", () => {
      const shouldOpen = !expandedDates.has(day);
      if (shouldOpen) expandedDates.add(day);
      else expandedDates.delete(day);
      dateGroup.classList.toggle("is-open", shouldOpen);
      heading.setAttribute("aria-expanded", String(shouldOpen));
      entries.setAttribute("aria-hidden", String(!shouldOpen));
      entries.inert = !shouldOpen;
    });
    const label = document.createElement("span");
    label.className = "date-label";
    label.textContent = dateFormatter.format(group.date);
    heading.append(label);
    const entries = document.createElement("div");
    entries.className = "date-entries";
    entries.id = entriesId;
    entries.setAttribute("aria-hidden", String(!isOpen));
    entries.inert = !isOpen;
    const entriesInner = document.createElement("div");
    entriesInner.className = "date-entries-inner";
    for (const { key, item } of group.items) entriesInner.append(makeTranscriptEntry(item, key));
    entries.append(entriesInner);
    dateGroup.append(heading, entries);
    fragment.append(dateGroup);
  }
  timeline.replaceChildren(fragment);
  updateHistoryControls();
  timeline.scrollTop = scrollToTop ? 0 : scrollTop;
  renderActivityCalendar();
}

function renderActivityCalendar() {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const todayKey = dateKey(today);
  const rangeStart = new Date(today);
  rangeStart.setDate(today.getDate() - 364);
  const rangeStartKey = dateKey(rangeStart);
  const firstWeek = new Date(rangeStart);
  firstWeek.setDate(rangeStart.getDate() - rangeStart.getDay());
  const dailyCounts = new Map();
  for (const item of transcripts.values()) {
    const date = transcriptDate(item);
    if (!date) continue;
    const day = dateKey(date);
    if (day >= rangeStartKey && day <= todayKey) dailyCounts.set(day, (dailyCounts.get(day) || 0) + 1);
  }
  const total = [...dailyCounts.values()].reduce((sum, value) => sum + value, 0);
  activityTotal.textContent = `${total} ${total === 1 ? "session" : "sessions"} in the past year`;
  activityCalendarDescription.setAttribute(
    "aria-label",
    `Daily transcription activity for the past year: ${total} ${total === 1 ? "session" : "sessions"}. Darker green squares indicate more sessions in a day.`,
  );
  activityCalendar.replaceChildren();
  activityMonths.replaceChildren();

  const monthFormatter = new Intl.DateTimeFormat(undefined, { month: "short" });
  const dateFormatter = new Intl.DateTimeFormat(undefined, { month: "long", day: "numeric", year: "numeric" });
  for (let week = 0; week < 53; week += 1) {
    const weekStart = new Date(firstWeek);
    weekStart.setDate(firstWeek.getDate() + week * 7);
    let monthStart = null;
    for (let weekday = 0; weekday < 7; weekday += 1) {
      const date = new Date(weekStart);
      date.setDate(weekStart.getDate() + weekday);
      if (date.getDate() === 1) {
        monthStart = date;
        break;
      }
    }
    if (week === 0 || monthStart) {
      const month = document.createElement("span");
      month.className = "activity-month";
      month.style.gridColumn = String(week + 1);
      month.textContent = monthFormatter.format(week === 0 ? rangeStart : monthStart);
      activityMonths.append(month);
    }
    const weekColumn = document.createElement("div");
    weekColumn.className = "activity-week";
    for (let weekday = 0; weekday < 7; weekday += 1) {
      const date = new Date(weekStart);
      date.setDate(weekStart.getDate() + weekday);
      const day = dateKey(date);
      const sessions = dailyCounts.get(day) || 0;
      const level = sessions === 0 ? 0 : sessions === 1 ? 1 : sessions <= 3 ? 2 : sessions <= 6 ? 3 : 4;
      const cell = document.createElement("span");
      cell.className = `activity-day level-${level}`;
      if (day === todayKey) cell.classList.add("is-today");
      if (day < rangeStartKey || day > todayKey) {
        cell.classList.add("is-outside-range");
      } else {
        cell.title = sessions === 0
          ? `No sessions on ${dateFormatter.format(date)}`
          : `${sessions} ${sessions === 1 ? "session" : "sessions"} on ${dateFormatter.format(date)}`;
      }
      weekColumn.append(cell);
    }
    activityCalendar.append(weekColumn);
  }
}

function replaceHistory(items) {
  const previousDays = new Set([...transcripts.values()].flatMap((item) => {
    const date = transcriptDate(item);
    return date ? [dateKey(date)] : [];
  }));
  const hadHistory = transcripts.size > 0;
  transcripts.clear();
  if (Array.isArray(items)) {
    for (const item of items) {
      if (!item || typeof item.text !== "string" || !transcriptDate(item)) continue;
      transcripts.set(`${item.timestamp}:${item.text}`, item);
    }
  }
  const newestDay = [...transcripts.values()]
    .map((item) => transcriptDate(item))
    .filter(Boolean)
    .map(dateKey)
    .sort((a, b) => b.localeCompare(a))[0];
  if (hadHistory && newestDay && !previousDays.has(newestDay)) expandedDates.add(newestDay);
  renderHistory();
}

function mergeHistory(items) {
  if (!Array.isArray(items)) return;
  for (const item of items) {
    if (!item || typeof item.text !== "string" || !transcriptDate(item)) continue;
    const key = `${item.timestamp}:${item.text}`;
    if (!transcripts.has(key)) transcripts.set(key, item);
  }
  renderHistory();
}

showState();

async function initializeDashboard() {
  // Await listener registration before requesting the initial snapshot. This
  // closes the startup gap where a fast transcription could be missed.
  if (listen) {
    await listen("dictation-state", ({ payload }) => showState(payload));
    await listen("transcription", ({ payload }) => addTranscript(payload, true));
    await listen("transcription-history", ({ payload }) => replaceHistory(payload));
  }
  if (!invoke) {
    historyRecordingToggle.disabled = false;
    return;
  }
  invoke("dashboard_history_recording").then((enabled) => {
    historyRecording = Boolean(enabled);
    historyRecordingToggle.checked = historyRecording;
    historyRecordingToggle.disabled = false;
  }).catch((error) => {
    historyRecordingToggle.disabled = false;
    historyStatus.textContent = `Could not read history setting: ${error}`;
  });
  invoke("dashboard_state").then(showState).catch((error) => {
    dashboard.dataset.modelState = "error";
    detail.textContent = "App status unavailable";
    detail.title = `Could not read app status: ${error}`;
  });
  invoke("dashboard_recognizer_settings").then((settings) => {
    if (modelDirectoryEdited) return;
    savedModelDirectory = settings.model_directory;
    modelDirectory.value = settings.model_directory;
    resizeModelDirectoryInput();
  }).catch(() => {});
  invoke("dashboard_hotkey").then((shortcut) => {
    if (!shortcutChanged) {
      savedShortcut = shortcut;
      if (!capturingShortcut) renderShortcutKeycaps(shortcut);
      shortcutHint.textContent = `Start dictating with ${shortcut}`;
    }
  }).catch(() => {});
  invoke("dashboard_history").then((history) => {
    mergeHistory(history);
  }).catch(() => {});
}

initializeDashboard();
window.addEventListener("resize", resizeModelDirectoryInput);

historyRecordingToggle.addEventListener("change", async () => {
  if (!invoke) return;
  const nextValue = historyRecordingToggle.checked;
  historyRecordingToggle.disabled = true;
  historyStatus.textContent = nextValue ? "Enabling history recording…" : "Stopping history recording…";
  try {
    await invoke("set_history_recording", { enabled: nextValue });
    historyRecording = nextValue;
    historyStatus.textContent = nextValue ? "New transcriptions will be saved." : "New transcriptions will not be saved.";
  } catch (error) {
    historyRecordingToggle.checked = historyRecording;
    historyStatus.textContent = `Could not update history setting: ${error}`;
  } finally {
    historyRecordingToggle.disabled = false;
  }
});

deleteHistoryButton.addEventListener("click", async () => {
  if (!invoke || transcripts.size === 0 || historyDeletePending) return;
  if (!window.confirm("Delete all saved transcription history? This cannot be undone.")) return;
  historyDeletePending = true;
  updateHistoryControls();
  historyStatus.textContent = "Deleting history…";
  try {
    await invoke("delete_history");
    transcripts.clear();
    expandedDates.clear();
    renderHistory();
    historyStatus.textContent = "History deleted.";
  } catch (error) {
    historyStatus.textContent = `Could not delete history: ${error}`;
  } finally {
    historyDeletePending = false;
    updateHistoryControls();
  }
});

modelDirectory.addEventListener("input", () => {
  modelDirectoryEdited = true;
  resizeModelDirectoryInput();
  scheduleModelDirectorySave();
});

modelDirectory.addEventListener("blur", (event) => {
  if (event.relatedTarget === pickModelDirectoryButton) {
    window.clearTimeout(modelSaveTimer);
  } else {
    scheduleModelDirectorySave(true);
  }
});

downloadButton.addEventListener("click", async () => {
  if (!invoke) return;
  downloadButton.disabled = true;
  modelError.textContent = "Loading the local speech model…";
  try {
    window.clearTimeout(modelSaveTimer);
    await modelSaveQueue;
    await invoke("save_recognizer_settings", { modelDirectory: modelDirectory.value });
    savedModelDirectory = modelDirectory.value.trim();
    modelActions.hidden = true;
  } catch (error) {
    modelError.textContent = String(error);
    downloadButton.disabled = false;
  }
});

pickModelDirectoryButton.addEventListener("click", async () => {
  if (!invoke) return;
  pickModelDirectoryButton.disabled = true;
  try {
    const directory = await invoke("pick_model_directory");
    if (directory) {
      modelDirectory.value = directory;
      modelDirectoryEdited = true;
      resizeModelDirectoryInput();
      scheduleModelDirectorySave(true);
    } else {
      scheduleModelDirectorySave(true);
    }
  } catch (error) {
    modelSettingsStatus.textContent = String(error);
  } finally {
    pickModelDirectoryButton.disabled = false;
  }
});

editHotkeyButton.addEventListener("click", () => {
  if (savingShortcut) return;
  if (capturingShortcut) {
    void cancelShortcutCapture();
    return;
  }
  void beginShortcutCapture();
});

window.addEventListener("keydown", (event) => {
  if (!capturingShortcut || savingShortcut || event.isComposing) return;
  if (event.key === "Escape" && !event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey) {
    event.preventDefault();
    void cancelShortcutCapture();
    return;
  }

  event.preventDefault();
  event.stopPropagation();
  const modifiers = shortcutModifiers(event);
  if (["Control", "Alt", "Shift", "Meta"].includes(event.key)) {
    renderShortcutCapture(modifiers);
    hotkeyFeedback.textContent = "Press one key · Esc to cancel";
    return;
  }

  const key = shortcutKeyName(event);
  renderShortcutCapture(modifiers, key || "");
  if (!key) {
    hotkeyFeedback.textContent = "Use a letter, number, function, or navigation key.";
    return;
  }
  if (modifiers.length === 0) {
    hotkeyFeedback.textContent = "Add a modifier to finish.";
    return;
  }
  hotkeyFeedback.textContent = "";
  void saveCapturedShortcut([...modifiers, key].join(" + "));
});

