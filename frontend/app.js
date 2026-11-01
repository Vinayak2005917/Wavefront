const pill = document.querySelector("#pill");
const canvas = document.querySelector("#waveform");
const toggleButton = document.querySelector("#toggle-dictation");
const playIcon = document.querySelector(".play-icon");
const context = canvas.getContext("2d", { alpha: true });
const listen = window.__TAURI__?.event?.listen;

const states = {
  checking: "loading",
  downloading: "loading",
  loading: "loading",
  ready: "ready",
  stopped: "ready",
  listening: "listening",
  processing: "processing",
  model_error: "error",
  error: "error",
  hotkey_error: "error",
  focus_changed: "error",
};

let samples = [];
let barLevels = [];

const BAR_COUNT = 28;
const BAR_WIDTH = 3;

function showState(data = {}) {
  const state = states[data.state] || "ready";
  pill.classList.remove("state-loading", "state-ready", "state-listening", "state-processing", "state-error");
  pill.classList.add(`state-${state}`);
  const listening = state === "listening";
  playIcon.textContent = listening ? "" : "▶";
  toggleButton.setAttribute("aria-label", listening ? "Stop dictation" : "Start dictation");
  toggleButton.title = listening ? "Stop dictation" : "Start dictation";
  toggleButton.disabled = !["ready", "listening", "processing", "stopped"].includes(data.state);
  const indicator = document.querySelector(".listen-indicator");
  indicator.setAttribute("aria-label", listening ? "Microphone is listening" : "Microphone inactive");
  indicator.title = listening ? "Microphone is listening" : "Microphone inactive";
}

toggleButton.addEventListener("click", async (event) => {
  event.stopPropagation();
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) return;
  toggleButton.disabled = true;
  try {
    await invoke("toggle_dictation");
  } catch (error) {
    console.error("Could not toggle dictation:", error);
  } finally {
    window.setTimeout(() => { toggleButton.disabled = false; }, 250);
  }
});

function drawWaveform(nextSamples = samples) {
  samples = Array.isArray(nextSamples) ? nextSamples : [];
  const rect = canvas.getBoundingClientRect();
  const dpr = Math.max(1, window.devicePixelRatio || 1);
  const pixelWidth = Math.max(1, Math.round(rect.width * dpr));
  const pixelHeight = Math.max(1, Math.round(rect.height * dpr));
  if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
    canvas.width = pixelWidth;
    canvas.height = pixelHeight;
  }

  context.setTransform(dpr, 0, 0, dpr, 0, 0);
  context.clearRect(0, 0, rect.width, rect.height);
  context.fillStyle = "#dedede";
  const middle = rect.height / 2;
  const maxHalfHeight = rect.height * 0.38;
  const sideInset = BAR_WIDTH * 0.08;

  // Samples arrive oldest to newest. Mapping the newest bar to the right edge
  // makes each incoming update move the visible timeline from right to left.
  for (let bar = 0; bar < BAR_COUNT; bar += 1) {
    const start = Math.floor((bar * samples.length) / BAR_COUNT);
    const end = Math.floor(((bar + 1) * samples.length) / BAR_COUNT);
    let squareSum = 0;
    for (let i = start; i < end; i += 1) squareSum += (samples[i] || 0) ** 2;
    const rms = end > start ? Math.sqrt(squareSum / (end - start)) : 0;
    // A gentle curve keeps quiet speech visible while preserving loudness changes.
    const target = Math.max(0, Math.min(1, Math.sqrt(rms) * 8));
    const previous = barLevels[bar] || 0;
    barLevels[bar] = previous + (target - previous) * (target > previous ? 0.8 : 0.35);

    const height = Math.max(1.2, barLevels[bar] * maxHalfHeight * 2);
    const x = sideInset + bar * (rect.width - BAR_WIDTH - sideInset * 2) / (BAR_COUNT - 1);
    const y = middle - height / 2;
    context.beginPath();
    context.roundRect(x, y, BAR_WIDTH, height, BAR_WIDTH / 2);
    context.fill();
  }
}

if (listen) {
  listen("dictation-state", ({ payload }) => showState(payload));
  listen("dictation-show", () => requestAnimationFrame(() => pill.classList.remove("is-entering", "is-exiting")));
  listen("dictation-hide", () => pill.classList.add("is-exiting"));
  listen("dictation-waveform", ({ payload }) => drawWaveform(payload?.samples));
}

window.addEventListener("resize", () => drawWaveform(samples));
showState({ state: "checking" });
drawWaveform();
