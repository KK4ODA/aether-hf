// Aether HF station panel.
//
// Talks to `aetherd` over the control API in `docs/spec/control-api.md`: a WebSocket at
// /v1 carrying requests correlated by id, and unsolicited events. Plain ES modules with no
// build step — see ADR-0005 — so the daemon can serve this directory as it stands and a
// gateway needs no Node toolchain to build.

const endpoint = () => {
  // Served by the daemon: talk to wherever we were loaded from. Opened as a file (the Tauri
  // shell, or a browser pointed at the checkout): fall back to the default loopback port.
  if (location.protocol === "http:" || location.protocol === "https:") {
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    return `${scheme}//${location.host}/v1`;
  }
  return "ws://127.0.0.1:8515/v1";
};

const $ = (id) => document.getElementById(id);

// ── connection ──────────────────────────────────────────────────────

const pending = new Map();
let socket = null;
let nextId = 1;
let reconnectDelay = 500;

function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      reject(new Error("not connected to the modem"));
      return;
    }
    const id = String(nextId++);
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
    // A modem that never answers must not leave a button disabled for ever.
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`${method} timed out`));
    }, 10000);
  });
}

function connect() {
  const url = endpoint();
  $("footer-endpoint").textContent = url;
  socket = new WebSocket(url);

  socket.addEventListener("open", async () => {
    reconnectDelay = 500;
    setLink(true);
    scopesWanted();
    log("connected to the modem");
    refreshStatus();
    loadCapabilities();
    loadHeard();
    loadMemories();
    // devices first: the configuration selects among them, and a profile's guess must not
    // overwrite what the file says
    await loadDevices();
    await loadConfig();
  });

  socket.addEventListener("message", (message) => {
    let frame;
    try {
      frame = JSON.parse(message.data);
    } catch {
      return;
    }
    if (frame.event !== undefined) {
      onEvent(frame);
      return;
    }
    const waiting = pending.get(frame.id);
    if (!waiting) return;
    pending.delete(frame.id);
    if (frame.ok) waiting.resolve(frame.result ?? {});
    else waiting.reject(new Error(frame.error?.message ?? "the modem refused"));
  });

  socket.addEventListener("close", () => {
    setLink(false);
    stopScopes();
    for (const [id, waiting] of pending) {
      waiting.reject(new Error("the connection closed"));
      pending.delete(id);
    }
    // Back off, but keep trying: a gateway's daemon restarting is a normal event and the
    // panel should come back on its own rather than needing a reload.
    setTimeout(connect, reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 10000);
  });

  socket.addEventListener("error", () => socket.close());
}

function setLink(up) {
  setLamp("lamp-link", up, up ? "Connected to the modem" : "Not connected to the modem");
  if (!up) {
    setState("offline", "Not connected", `No modem at ${endpoint()}. Retrying.`);
    $("footer-version").textContent = "not connected";
    setLamp("lamp-ptt", false, "Transmitter off");
    setLamp("lamp-rx", false, "Nothing arriving");
    setLamp("lamp-busy", false, "Channel clear");
  }
  for (const id of [
    "btn-connect",
    "btn-disconnect",
    "btn-abort",
    "btn-beacon",
    "btn-probe",
    "btn-send",
    "btn-record",
    "btn-tune-to",
    "btn-memory-add",
    "btn-memory-remove",
  ]) {
    $(id).disabled = !up;
  }
}

// ── events ──────────────────────────────────────────────────────────

function onEvent(frame) {
  const data = frame.data ?? {};
  switch (frame.event) {
    case "metrics":
      applyMetrics(data);
      break;
    case "ptt":
      setLamp("lamp-ptt", data.on === true, data.on === true ? "Transmitter keyed" : "Transmitter off");
      break;
    case "state":
      log(`${data.name}: ${data.detail}`);
      if (data.name === "connected") resetReceived();
      refreshStatus();
      break;
    case "frame":
      onFrame(data);
      break;
    case "heard":
      noteHeard(data);
      break;
    case "data":
      onReceivedData(data.data ?? "");
      break;
    case "log":
      log(`${data.name}: ${data.detail}`, data.name === "error");
      // the probe's answer, or its absence, where the button is
      if (data.name === "probe") noteProbe(data.detail ?? "");
      if (data.name === "test") noteTest(data.detail ?? "");
      break;
    default:
      log(`${frame.event}: ${JSON.stringify(data)}`);
  }
}

// The probe's report is the modem's own sentence — "KK4XYZ hears us at 12 dB, heard at
// 14.0 dB" or "KK4XYZ: no answer" — shown as it is, beside the button that asked.
function noteTest(detail) {
  const line = $("test-result");
  line.textContent = `Test session: ${detail}`;
  line.dataset.state = detail.startsWith("aborted") || detail.startsWith("stopped") ? "warn" : "ok";
}

function noteProbe(detail) {
  const line = $("probe-result");
  const unanswered = detail.endsWith("no answer");
  line.textContent = unanswered ? `${detail} — try again, or change band` : detail;
  line.dataset.state = unanswered ? "warn" : "ok";
}

// ── status ──────────────────────────────────────────────────────────

const STATE_TEXT = {
  idle: ["Idle", "Listening. No session."],
  connecting: ["Calling", "Waiting for the other station to answer."],
  connected: ["Connected", "Session up."],
  disconnecting: ["Closing", "Finishing what is queued, then closing."],
};

function setState(key, name, detail) {
  $("state-strip").dataset.state = key;
  $("state-name").textContent = name;
  $("state-detail").textContent = detail;
}

// The status the panel last saw: whether a restart is something it can do for the operator.
let lastStatus = null;

async function refreshStatus() {
  let status;
  try {
    status = await call("status");
  } catch {
    return;
  }
  lastStatus = status;
  const [name, detail] = STATE_TEXT[status.state] ?? ["Unknown", ""];
  const who = status.remote ? ` with ${status.remote}` : "";
  const role = { iss: " — sending", irs: " — receiving" }[status.role] ?? "";
  setState(status.state, name + who, detail + role);

  $("callsign").textContent = status.callsign || "—";
  // a first run: the configuration still has the placeholder callsign, so the wizard is
  // where this panel should open — once, and only until Setup is saved
  if (!firstRunShown && status.callsign === "N0CALL") {
    firstRunShown = true;
    selectTab($("tab-setup"));
  }
  $("footer-version").textContent = `aetherd ${status.version} · ptt: ${status.ptt}`;
  $("help-daemon").textContent = `aetherd ${status.version}`;
  $("help-callsign").textContent = status.callsign || "—";
  $("help-ptt").textContent = status.ptt;
  setLamp("lamp-ptt", status.transmitting === true, status.transmitting ? "Transmitter keyed" : "Transmitter off");
  setLamp("lamp-busy", status.channel_busy === true, status.channel_busy ? "Channel busy" : "Channel clear");

  $("v-queued").textContent = String(status.queued_bytes ?? 0);
  const saving = status.compression_saving ?? 0;
  $("v-compress-sub").textContent = status.compressing
    ? `bytes to send · compressed ${Math.round(saving * 100)}%`
    : "bytes still to send";

  const dial = $("dial");
  if (status.frequency_hz) {
    dial.textContent = formatHz(status.frequency_hz);
    dial.hidden = false;
  } else {
    dial.hidden = true;
  }
  const host = status.host ?? { enabled: false, connected: false };
  $("d-host").textContent = host.enabled ? (host.connected ? "connected" : "listening") : "off";
  $("d-host-sub").textContent = host.enabled
    ? `${host.command_address ?? ""} · data ${host.data_address ?? ""}`
    : "turn on Host programs in Setup";

  applyMetrics(status.metrics ?? {});
  renderCounters(status.counters ?? {});

  applyDial(status);
  $("btn-connect").disabled = status.state !== "idle";
  $("btn-beacon").disabled = status.state !== "idle";
  $("btn-probe").disabled = status.state !== "idle";
  // one button: it starts a test session when the station is idle, and stops the one
  // that is running
  $("btn-test").textContent = status.test ? "Stop test" : "Test session";
  $("btn-test").disabled = !status.test && status.state !== "idle";
  testRunning = Boolean(status.test);
  if (status.test) {
    const t = status.test;
    $("test-result").textContent =
      `Test session with ${t.remote}: ${t.step} — ${Math.round(t.elapsed_s)} s, ${t.rungs} rung${t.rungs === 1 ? "" : "s"}`;
  }
  applyRecording(status.recording ?? null);
  applyRecordingsDir(status.recordings_dir ?? null);
  applyLastSession(status);
  // a fresh call must not leave the last probe's or test's outcome on the panel: a
  // stale "no answer" sitting through a session that then succeeds is exactly the
  // confusion reported after the first radio-to-radio test
  if (status.state !== "idle") {
    clearNote("probe-result");
    if (!testRunning) clearNote("test-result");
  }
  $("btn-disconnect").disabled = status.state === "idle";
  $("btn-abort").disabled = status.state === "idle";
  $("btn-send").disabled = status.state !== "connected";
  $("send-note").textContent =
    status.state === "connected" ? "" : "Connect to a station first.";
}

let modeTable = [];

let currentMode = null;

function applyMetrics(metrics) {
  const mode = metrics.mode;
  if (mode !== undefined) {
    currentMode = mode;
    $("v-mode").textContent = String(mode);
    const entry = modeTable[mode];
    $("v-mode-name").textContent = entry
      ? `${entry.name}${entry.floor ? " (floor)" : ""} · ${Math.round(entry.net_bit_rate)} bit/s`
      : "";
  }
  if (metrics.queued_bytes !== undefined) {
    $("v-queued").textContent = String(metrics.queued_bytes);
  }
  // the link: the receiver's last frame, what the other end reports, the account
  if (metrics.snr_db !== undefined) {
    const snr = metrics.snr_db;
    const peer = metrics.peer_snr_db;
    $("v-snr").textContent = snr === null ? "—" : `${snr.toFixed(1)} dB`;
    $("v-snr-sub").textContent =
      peer !== null && peer !== undefined
        ? `they hear you at ${peer.toFixed(1)} dB`
        : snr === null
          ? "no frame heard yet"
          : "on the last frame heard";
    $("d-peer").textContent = peer === null || peer === undefined ? "—" : `${peer.toFixed(1)} dB`;
    if (peer !== null && peer !== undefined) notePeer(peer);
  }
  // a null offset is a noise trigger the modem would not vouch for: hold the last real
  // reading rather than blank it
  if (typeof metrics.cfo_hz === "number") applyOffset(metrics.cfo_hz);
  if (metrics.throughput_bps !== undefined) {
    $("v-throughput").textContent = metrics.link ? formatRate(metrics.throughput_bps) : "—";
    const at = Date.now();
    // speed is zero with no session; the mode's on-air rate is only meaningful in one
    const rate = metrics.link ? (modeTable[currentMode]?.net_bit_rate ?? null) : null;
    throughputHistory.push({ at, bps: metrics.link ? metrics.throughput_bps : 0, rate });
    while (throughputHistory.length && throughputHistory[0].at < at - SNR_SPAN_MS) throughputHistory.shift();
    if (throughputHistory.length > 1400) throughputHistory.shift();
  }
  if (metrics.link !== undefined) applyLink(metrics.link);
  if (metrics.rate_snr_db !== undefined) {
    const smoothed = metrics.rate_snr_db;
    $("d-rate").textContent = smoothed === null ? "—" : `${smoothed.toFixed(1)} dB`;
    $("d-rate-sub").textContent =
      smoothed === null
        ? "nothing measured this session"
        : `smoothed · margin ${metrics.margin_db.toFixed(1)} dB`;
  }
  if (metrics.receiving !== undefined) {
    setLamp("lamp-rx", metrics.receiving === true, metrics.receiving ? "A burst is arriving" : "Nothing arriving");
  }
  if (metrics.transmitting !== undefined && metrics.receiving !== undefined) {
  }
  const level = metrics.level_db;
  const floor = metrics.noise_floor_db;
  if (level === null || level === undefined || floor === null || floor === undefined) {
    // Null is "not measured yet", not "quiet": the detector needs several seconds of audio
    // before its floor means anything, and drawing a zero would be a lie.
    $("v-excess").textContent = "—";
    $("v-floor").textContent = "still listening";
  } else {
    $("v-excess").textContent = `${(level - floor).toFixed(1)} dB`;
    $("v-floor").textContent = `floor ${floor.toFixed(1)} dBFS`;
    // while this station transmits the receiver is muted and the detector holds its
    // last reading, which is not the channel: the sample says so instead of carrying it
    const at = Date.now();
    history.push({
      at,
      level: metrics.transmitting === true ? null : level,
      floor,
      tx: metrics.transmitting === true,
      rx: metrics.receiving === true,
    });
    while (history.length && history[0].at < at - LEVEL_SPAN_MS) history.shift();
    drawChart();
    // the SNR chart's window slides with the clock even when no frame arrives
    drawStatusChart();
  }
  if (level !== null && level !== undefined && floor !== null && floor !== undefined) {
    $("d-busy").textContent = metrics.channel_busy ? "busy" : "clear";
    $("d-busy-sub").textContent = `level ${level.toFixed(1)} · floor ${floor.toFixed(1)} dBFS`;
  }
  setLamp("lamp-busy", metrics.channel_busy === true, metrics.channel_busy ? "Channel busy" : "Channel clear");
  if (metrics.transmitting !== undefined) {
    setLamp("lamp-ptt", metrics.transmitting === true, metrics.transmitting ? "Transmitter keyed" : "Transmitter off");
  }
  if (metrics.audio !== undefined) {
    updateMeter(metrics.audio);
    applyRxLevel(metrics.audio);
  }
}

function applyOffset(cfo) {
  if (cfo === null || cfo === undefined) {
    $("v-cfo").textContent = "—";
    $("v-cfo-sub").textContent = "offset of the last frame";
    return;
  }
  const rounded = Math.round(cfo);
  const sign = rounded > 0 ? "+" : rounded < 0 ? "−" : "";
  $("v-cfo").textContent = `${sign}${Math.abs(rounded)} Hz`;
  $("v-cfo-sub").textContent =
    Math.abs(cfo) < 3
      ? "on frequency"
      : cfo > 0
        ? "the other station is high"
        : "the other station is low";
}

function applyRxLevel(audio) {
  if (!audio.settled) {
    $("v-rx-level").textContent = "—";
    $("v-rx-level-sub").textContent = "still listening";
    $("d-audio").textContent = "—";
    return;
  }
  $("v-rx-level").textContent = `${audio.rms_dbfs.toFixed(0)} dBFS`;
  $("v-rx-level-sub").textContent = audio.clipping > 0.001
    ? "clipping: turn the receive level down"
    : `peak ${audio.peak_dbfs.toFixed(0)} dBFS`;
  $("d-audio").textContent = `${audio.rms_dbfs.toFixed(1)} dBFS`;
  $("d-audio-sub").textContent =
    `peak ${audio.peak_dbfs.toFixed(1)} dBFS · clipping ${(audio.clipping * 100).toFixed(2)}%`;
}

function applyLink(link) {
  if (!link) {
    $("v-session").textContent = "—";
    $("v-session-sub").textContent = "no session";
    $("v-bytes").textContent = "no session";
    $("v-throughput").textContent = "—";
    return;
  }
  $("v-session").textContent = formatDuration(link.seconds ?? 0);
  const role = { iss: " · sending", irs: " · receiving" }[lastStatus?.role] ?? "";
  $("v-session-sub").textContent = `with ${link.remote ?? "—"}${role}`;
  $("v-bytes").textContent =
    `↑ ${formatBytes(link.bytes_sent ?? 0)} · ↓ ${formatBytes(link.bytes_received ?? 0)}`;
}

// ── formatting ──────────────────────────────────────────────────────

const pad2 = (n) => String(n).padStart(2, "0");

function formatDuration(seconds) {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  return h ? `${h}:${pad2(m)}:${pad2(r)}` : `${m}:${pad2(r)}`;
}

function formatBytes(n) {
  if (n < 1000) return `${n} B`;
  if (n < 1e6) return `${(n / 1000).toFixed(1)} kB`;
  return `${(n / 1e6).toFixed(2)} MB`;
}

function formatRate(bps) {
  return bps >= 1000 ? `${(bps / 1000).toFixed(2)} kbit/s` : `${Math.round(bps)} bit/s`;
}

/// 14107000 → "14.107.000", the way a rig's display groups it.
function formatHz(hz) {
  return String(Math.round(hz)).replace(/\B(?=(\d{3})+(?!\d))/g, ".");
}

function relative(ms, now = Date.now()) {
  const d = Math.max(0, now - ms) / 1000;
  if (d < 45) return "just now";
  if (d < 3600) return `${Math.max(1, Math.floor(d / 60))} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  return `${Math.floor(d / 86400)} d ago`;
}

function clock(ms) {
  const d = new Date(ms);
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`;
}

function stamp(ms) {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${clock(ms)}`;
}

// ── frames: the receiver's own readings, one per frame ──────────────

const SNR_SPAN_MS = 10 * 60 * 1000;
const snrHistory = [];
// connection speed over the same ten-minute window: {at, bps goodput, rate mode on-air}
const throughputHistory = [];
const peerHistory = [];
const framesSeen = [];
let lastPeer = null;

// The readings survive a reload of the page: the chart's ten minutes of SNR, what the
// other station reported, and the last frames — kept per browser, and dropped once
// they are older than the chart shows.
let testRunning = false;
let firstRunShown = false;
const HISTORY_KEY = "aether.history";
let historySaveTimer = null;

function loadHistory() {
  try {
    const kept = JSON.parse(localStorage.getItem(HISTORY_KEY) ?? "null");
    if (!kept) return;
    const oldest = Date.now() - SNR_SPAN_MS;
    for (const point of kept.snr ?? []) if (point.at >= oldest) snrHistory.push(point);
    for (const point of kept.peer ?? []) if (point.at >= oldest) peerHistory.push(point);
    for (const frame of kept.frames ?? []) if (frame.at >= oldest) framesSeen.push(frame);
    lastPeer = peerHistory.at(-1)?.snr ?? null;
  } catch {
    // nothing kept, or nothing readable: the chart starts empty, as it always did
  }
}

function saveHistory() {
  if (historySaveTimer) return;
  historySaveTimer = setTimeout(() => {
    historySaveTimer = null;
    try {
      localStorage.setItem(
        HISTORY_KEY,
        JSON.stringify({ snr: snrHistory, peer: peerHistory, frames: framesSeen }),
      );
    } catch {
      // a browser that keeps nothing keeps nothing
    }
  }, 2000);
}

function onFrame(frame) {
  const at = Date.now();
  snrHistory.push({ at, snr: frame.snr_db, decoded: frame.decoded === true, kind: frame.kind });
  while (snrHistory.length && snrHistory[0].at < at - SNR_SPAN_MS) snrHistory.shift();
  if (snrHistory.length > 1200) snrHistory.shift();
  framesSeen.unshift({ at, ...frame });
  if (framesSeen.length > 60) framesSeen.pop();
  $("d-confidence").textContent =
    frame.confidence === undefined ? "—" : Number(frame.confidence).toFixed(2);
  if (typeof frame.cfo_hz === "number") applyOffset(frame.cfo_hz);
  if (frame.snr_db !== undefined) $("v-snr").textContent = `${Number(frame.snr_db).toFixed(1)} dB`;
  saveHistory();
  if (panelShown("status")) drawStatusChart();
  if (panelShown("diagnostics")) {
    renderFrames();
    fetchConstellation();
  }
}

function notePeer(snr) {
  if (snr === lastPeer) return;
  lastPeer = snr;
  const at = Date.now();
  peerHistory.push({ at, snr });
  while (peerHistory.length && peerHistory[0].at < at - SNR_SPAN_MS) peerHistory.shift();
  saveHistory();
}

function panelShown(name) {
  return !$(`panel-${name}`).hidden && document.visibilityState === "visible";
}

function tokens() {
  const style = getComputedStyle(document.body);
  const read = (name, fallback) => style.getPropertyValue(name).trim() || fallback;
  return {
    ink: read("--text-3", "#888"),
    ink2: read("--text-2", "#aaa"),
    accent: read("--accent", "#2dd4bf"),
    grid: read("--plot-grid", "#223"),
    tx: read("--status-tx", "#f59e0b"),
    rx: read("--status-rx", "#22d3ee"),
    error: read("--status-error", "#f87171"),
    plot: read("--plot-bg", "#000"),
    numerals: read("--numerals", "monospace"),
  };
}

/// A canvas sized to its box at the device's pixel ratio; returns the drawing context and
/// the size in CSS pixels.
function surface(id, fallbackWidth, height) {
  const canvas = $(id);
  const ratio = window.devicePixelRatio || 1;
  const width = canvas.clientWidth || fallbackWidth;
  if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
  }
  const ctx = canvas.getContext("2d");
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  return { ctx, width, height };
}

// ── the Status tab's two charts share one frame ─────────────────────
//
// The same margins, type, grid and legend line, so the two read as parts of one
// dashboard: the y axis is labelled in its unit at the head of the legend, the grid is
// faint, and the time axis carries three labels, not a ruler.

const CHART_LEFT = 40;
const CHART_RIGHT = 8;
const CHART_TOP = 20;
const CHART_BOTTOM = 20;
const LEVEL_SPAN_MS = 2 * 60 * 1000;

/// A colour token with an alpha, for the quieter marks.
function withAlpha(colour, alpha) {
  const m = /^#([0-9a-f]{6})$/i.exec(colour.trim());
  if (!m) return colour;
  const v = parseInt(m[1], 16);
  return `rgba(${(v >> 16) & 255}, ${(v >> 8) & 255}, ${v & 255}, ${alpha})`;
}

/// The frame both charts draw in: the surface, the tokens, and the y scale.
function chartFrame(id, fallbackWidth, height, low, high) {
  const s = surface(id, fallbackWidth, height);
  const c = tokens();
  s.ctx.clearRect(0, 0, s.width, s.height);
  s.ctx.font = `10.5px ${c.numerals}`;
  s.ctx.textBaseline = "middle";
  const plotWidth = s.width - CHART_LEFT - CHART_RIGHT;
  const bottom = height - CHART_BOTTOM;
  const y = (value) => CHART_TOP + (1 - (value - low) / (high - low)) * (bottom - CHART_TOP);
  return { ...s, c, plotWidth, bottom, y };
}

/// Horizontal grid lines every `step`, labelled at the left.
function drawValueAxis(f, low, high, step) {
  const { ctx, c } = f;
  ctx.lineWidth = 1;
  ctx.textAlign = "right";
  for (let value = Math.ceil(low / step) * step; value <= high; value += step) {
    const at = f.y(value);
    ctx.strokeStyle = c.grid;
    ctx.beginPath();
    ctx.moveTo(CHART_LEFT, at);
    ctx.lineTo(CHART_LEFT + f.plotWidth, at);
    ctx.stroke();
    ctx.fillStyle = c.ink;
    ctx.fillText(String(value), CHART_LEFT - 6, at);
  }
}

/// A faint rule every `gridMs`, and a label at every `labelMs`: "−N min" and "now".
function drawTimeAxis(f, now, spanMs, gridMs, labelMs) {
  const { ctx, c, height } = f;
  const x = (at) => CHART_LEFT + ((at - (now - spanMs)) / spanMs) * f.plotWidth;
  ctx.lineWidth = 1;
  ctx.strokeStyle = c.grid;
  for (let back = gridMs; back < spanMs; back += gridMs) {
    ctx.beginPath();
    ctx.moveTo(x(now - back), CHART_TOP);
    ctx.lineTo(x(now - back), f.bottom);
    ctx.stroke();
  }
  ctx.fillStyle = c.ink;
  for (let back = 0; back <= spanMs; back += labelMs) {
    ctx.textAlign = back === 0 ? "right" : back === spanMs ? "left" : "center";
    const minutes = back / 60_000;
    ctx.fillText(back === 0 ? "now" : `−${minutes} min`, x(now - back), height - 8);
  }
  return x;
}

/// The legend line along the top: the unit first, then each series in its colour.
function drawLegend(f, items) {
  const { ctx } = f;
  ctx.textAlign = "left";
  let at = CHART_LEFT;
  for (const [colour, text] of items) {
    ctx.fillStyle = colour;
    ctx.fillText(text, at, 8);
    at += ctx.measureText(text).width + 14;
  }
}

// ── the Status chart area: SNR by frame, or connection speed ────────
//
// One frame holds two views the operator switches between. Only the visible canvas is
// drawn — a hidden one has no width to size against — so every redraw goes through
// drawStatusChart.

let statusChart = "snr";

function drawStatusChart() {
  if (statusChart === "speed") drawSpeedChart();
  else drawSnrChart();
}

function selectStatusChart(which) {
  statusChart = which === "speed" ? "speed" : "snr";
  const speed = statusChart === "speed";
  $("chart-snr").hidden = speed;
  $("chart-speed").hidden = !speed;
  $("chart-tab-snr").setAttribute("aria-selected", String(!speed));
  $("chart-tab-speed").setAttribute("aria-selected", String(speed));
  try {
    localStorage.setItem("aether.statuschart", statusChart);
  } catch {
    // a browser that keeps nothing keeps nothing
  }
  drawStatusChart();
}

// ── connection speed, last ten minutes ──────────────────────────────
//
// The goodput the session is moving, both ways, against the on-air rate of the mode it
// is running — so a curve well under the mode line means the link is the limit, not the
// modem, and a curve near it means the mode is working as fast as it can.

function drawSpeedChart() {
  const now = Date.now();
  let high = 200;
  for (const point of throughputHistory) {
    high = Math.max(high, point.bps, point.rate ?? 0);
  }
  const step = high > 4000 ? 1000 : high > 1500 ? 500 : 100;
  high = Math.ceil(high / step) * step;
  const f = chartFrame("chart-speed", 450, 180, 0, high);
  const { ctx, c } = f;
  drawValueAxis(f, 0, high, step);
  const x = drawTimeAxis(f, now, SNR_SPAN_MS, 2 * 60_000, 5 * 60_000);
  drawLegend(f, [[c.ink, "bit/s"], [withAlpha(c.tx, 0.9), "mode rate"], [c.accent, "goodput"]]);

  // the mode's on-air rate: a faint dashed step, the ceiling the goodput works under
  ctx.strokeStyle = withAlpha(c.tx, 0.8);
  ctx.lineWidth = 1;
  ctx.setLineDash([2, 3]);
  ctx.beginPath();
  let started = false;
  for (const point of throughputHistory) {
    if (point.rate == null) {
      started = false;
      continue;
    }
    if (started) ctx.lineTo(x(point.at), f.y(point.rate));
    else ctx.moveTo(x(point.at), f.y(point.rate));
    started = true;
  }
  ctx.stroke();
  ctx.setLineDash([]);

  // the goodput itself: the solid accent line, the reading that matters
  ctx.strokeStyle = c.accent;
  ctx.lineWidth = 1.5;
  ctx.lineJoin = "round";
  ctx.beginPath();
  throughputHistory.forEach((point, index) => {
    if (index === 0) ctx.moveTo(x(point.at), f.y(point.bps));
    else ctx.lineTo(x(point.at), f.y(point.bps));
  });
  ctx.stroke();
}

// ── SNR by frame, last ten minutes ──────────────────────────────────

function drawSnrChart() {
  const now = Date.now();
  const threshold = modeTable[currentMode]?.threshold_db;
  let low = -2;
  let high = 20;
  for (const point of [...snrHistory, ...peerHistory]) {
    low = Math.min(low, point.snr - 2);
    high = Math.max(high, point.snr + 2);
  }
  if (threshold !== undefined) {
    low = Math.min(low, threshold - 2);
    high = Math.max(high, threshold + 2);
  }
  low = Math.floor(low / 5) * 5;
  high = Math.ceil(high / 5) * 5;
  const f = chartFrame("chart-snr", 450, 180, low, high);
  const { ctx, c } = f;
  drawValueAxis(f, low, high, high - low > 40 ? 10 : 5);
  const x = drawTimeAxis(f, now, SNR_SPAN_MS, 2 * 60_000, 5 * 60_000);

  if (threshold !== undefined) {
    ctx.strokeStyle = withAlpha(c.ink, 0.7);
    ctx.setLineDash([3, 4]);
    ctx.beginPath();
    ctx.moveTo(CHART_LEFT, f.y(threshold));
    ctx.lineTo(CHART_LEFT + f.plotWidth, f.y(threshold));
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.textAlign = "left";
    ctx.fillStyle = c.ink2;
    ctx.fillText(
      `mode ${currentMode} needs ${threshold.toFixed(1)} dB`,
      CHART_LEFT + 4,
      f.y(threshold) - 7,
    );
  }

  // what the other station reports hearing us at: a thin dashed line with small squares,
  // quieter than the receiver's own readings so the two are told apart at a glance
  ctx.strokeStyle = withAlpha(c.tx, 0.8);
  ctx.lineWidth = 1;
  ctx.setLineDash([2, 3]);
  ctx.beginPath();
  peerHistory.forEach((point, index) => {
    if (index === 0) ctx.moveTo(x(point.at), f.y(point.snr));
    else ctx.lineTo(x(point.at), f.y(point.snr));
  });
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.fillStyle = c.tx;
  for (const point of peerHistory) {
    ctx.fillRect(x(point.at) - 1.5, f.y(point.snr) - 1.5, 3, 3);
  }

  // the receiver's own: a line through the decoded frames with a small dot each, and a
  // hollow mark for a frame that did not decode — the reading that matters most
  ctx.strokeStyle = c.accent;
  ctx.lineWidth = 1.25;
  ctx.lineJoin = "round";
  ctx.beginPath();
  let started = false;
  for (const point of snrHistory) {
    if (!point.decoded) continue;
    if (started) ctx.lineTo(x(point.at), f.y(point.snr));
    else ctx.moveTo(x(point.at), f.y(point.snr));
    started = true;
  }
  ctx.stroke();
  for (const point of snrHistory) {
    ctx.beginPath();
    if (point.decoded) {
      ctx.arc(x(point.at), f.y(point.snr), 1.75, 0, Math.PI * 2);
      ctx.fillStyle = c.accent;
      ctx.fill();
    } else {
      ctx.arc(x(point.at), f.y(point.snr), 3, 0, Math.PI * 2);
      ctx.strokeStyle = c.error;
      ctx.lineWidth = 1.5;
      ctx.stroke();
    }
  }
  drawLegend(f, [
    [c.ink2, "SNR, dB"],
    [c.accent, "● heard here"],
    [c.error, "○ not decoded"],
    [c.tx, "■ they hear you"],
  ]);
}

// ── the stations heard ──────────────────────────────────────────────

let heardList = [];
let heardSort = { key: "last_heard_ms", direction: -1 };
const ACTIVITY_TEXT = {
  beacon: "beacon",
  calling: "calling",
  probing: "probing",
  answering: "answering",
  connected: "session",
};

async function loadHeard() {
  try {
    const result = await call("heard.list");
    heardList = result.stations ?? [];
  } catch {
    return;
  }
  renderHeard();
}

// ── the dial memories ───────────────────────────────────────────────
//
// The modem keeps the list (`frequencies.list`/`frequencies.set`), so it is the same from
// every panel and survives a reinstall; the panel only edits it whole. The select follows
// the radio's own dial when the dial changes and matches an entry, and otherwise keeps the
// operator's choice, so a pick made before Tune is not undone by the next status tick.

let memories = [];
let memoriesLoaded = false;
let lastDialHz = null;
let canTune = false;
let lastState = null;

function memoryLabel(entry) {
  const mhz = (entry.hz / 1e6).toFixed(entry.hz % 1000 === 0 ? 3 : 4);
  return entry.name ? `${mhz} MHz — ${entry.name}` : `${mhz} MHz`;
}

function renderMemories(selectHz = null) {
  const select = $("memory");
  const chosen = selectHz ?? Number(select.value) ?? null;
  select.replaceChildren();
  if (memories.length === 0) {
    const option = document.createElement("option");
    option.value = "";
    option.textContent = "no dial remembered yet — Add one";
    select.append(option);
  }
  for (const entry of memories) {
    const option = document.createElement("option");
    option.value = String(entry.hz);
    option.textContent = memoryLabel(entry);
    select.append(option);
  }
  if (chosen && memories.some((m) => m.hz === chosen)) select.value = String(chosen);
  $("btn-memory-remove").disabled = memories.length === 0;
  updateTuneButton();
}

function updateTuneButton() {
  const idle = lastState === null || lastState === "idle";
  $("btn-tune-to").disabled = !canTune || !idle || memories.length === 0;
}

async function loadMemories() {
  try {
    const result = await call("frequencies.list");
    memories = result.memories ?? [];
    memoriesLoaded = true;
  } catch {
    return;
  }
  renderMemories();
}

async function saveMemories(next, description) {
  try {
    const result = await call("frequencies.set", { memories: next });
    memories = result.memories ?? next;
    $("memory-note").textContent = description;
    delete $("memory-note").dataset.state;
    log(description);
    return true;
  } catch (error) {
    $("memory-note").textContent = error.message;
    $("memory-note").dataset.state = "error";
    log(error.message, true);
    return false;
  }
}

function applyDial(status) {
  canTune = status.can_tune === true;
  lastState = status.state ?? null;
  // a first request lost to a slow start is asked again with the next status
  if (!memoriesLoaded) loadMemories();
  const reading = $("dial-reading");
  if (status.frequency_hz) {
    reading.textContent = `radio: ${formatHz(status.frequency_hz)} Hz`;
    if (status.frequency_hz !== lastDialHz) {
      lastDialHz = status.frequency_hz;
      const match = memories.find((m) => Math.abs(m.hz - status.frequency_hz) <= 10);
      if (match) $("memory").value = String(match.hz);
    }
  } else if (canTune) {
    reading.textContent = "radio: no reading yet";
  } else {
    reading.textContent = "tuning needs CAT or rigctld keying (Setup step 2)";
  }
  updateTuneButton();
}

function parseMhz(text) {
  const value = Number(String(text).trim().replace(",", "."));
  if (!Number.isFinite(value) || value <= 0) return null;
  // a dial typed in kHz is still a dial: 14107 is 14.107
  const hz = value >= 1000 ? value * 1e3 : value * 1e6;
  return Math.round(hz);
}

function openMemoryForm() {
  const form = $("memory-form");
  form.hidden = false;
  $("memory-mhz").value = lastDialHz ? (lastDialHz / 1e6).toFixed(lastDialHz % 1000 === 0 ? 3 : 4) : "";
  $("memory-name").value = "";
  $("memory-note").textContent = "";
  delete $("memory-note").dataset.state;
  $("memory-mhz").focus();
}

function closeMemoryForm() {
  $("memory-form").hidden = true;
}

async function saveMemoryForm() {
  const hz = parseMhz($("memory-mhz").value);
  if (!hz) {
    $("memory-note").textContent = "a frequency in MHz is needed, such as 14.107";
    $("memory-note").dataset.state = "error";
    $("memory-mhz").focus();
    return;
  }
  const name = $("memory-name").value.trim();
  const next = memories.filter((m) => m.hz !== hz).concat([{ hz, name }]);
  const renamed = memories.some((m) => m.hz === hz);
  const ok = await saveMemories(next, `${renamed ? "renamed" : "remembered"} ${formatHz(hz)} Hz${name ? ` — ${name}` : ""}`);
  if (!ok) return;
  closeMemoryForm();
  renderMemories(hz);
}

async function removeMemory() {
  const hz = Number($("memory").value);
  const entry = memories.find((m) => m.hz === hz);
  if (!entry) return;
  const next = memories.filter((m) => m.hz !== hz);
  const ok = await saveMemories(next, `forgot ${memoryLabel(entry)}`);
  if (!ok) return;
  renderMemories();
}

async function tuneToMemory() {
  const hz = Number($("memory").value);
  if (!hz) return;
  const entry = memories.find((m) => m.hz === hz);
  const ok = await act(() => call("frequency.set", { hz }), `tuned to ${entry ? memoryLabel(entry) : `${formatHz(hz)} Hz`}`);
  if (ok) {
    $("dial-reading").textContent = `radio: ${formatHz(hz)} Hz`;
    lastDialHz = hz;
  }
}

function noteHeard(entry) {
  if (!entry || !entry.callsign) return;
  const index = heardList.findIndex((s) => s.callsign === entry.callsign);
  if (index >= 0) heardList[index] = entry;
  else heardList.push(entry);
  renderHeard();
}

function renderHeard() {
  const count = $("heard-count");
  count.textContent = String(heardList.length);
  count.hidden = heardList.length === 0;
  $("heard-empty").hidden = heardList.length > 0;
  $("heard-table").parentElement.hidden = heardList.length === 0;
  $("btn-heard-clear").disabled = heardList.length === 0;
  const { key, direction } = heardSort;
  const rows = [...heardList].sort((a, b) => {
    const p = a[key] ?? (typeof b[key] === "number" ? -Infinity : "");
    const q = b[key] ?? (typeof a[key] === "number" ? -Infinity : "");
    if (p === q) return b.last_heard_ms - a.last_heard_ms;
    return (p < q ? -1 : 1) * direction;
  });
  const now = Date.now();
  const body = $("heard");
  body.replaceChildren();
  for (const station of rows) {
    const row = document.createElement("tr");
    const cell = (text, className, title) => {
      const td = document.createElement("td");
      if (className) td.className = className;
      if (title) td.title = title;
      if (text instanceof Node) td.append(text);
      else td.textContent = text;
      row.append(td);
      return td;
    };
    cell(station.callsign, "call");
    cell(relative(station.last_heard_ms, now), "", stamp(station.last_heard_ms));
    cell(stamp(station.first_heard_ms), "numerals");
    cell(`${station.snr_db.toFixed(1)} dB`, "num");
    cell(`${station.best_snr_db.toFixed(1)} dB`, "num");
    cell(station.frequency_hz ? formatHz(station.frequency_hz) : "—", "num");
    const mode = modeTable[station.mode];
    cell(String(station.mode), "num", mode ? mode.name : "");
    const act = document.createElement("span");
    act.className = "act";
    act.dataset.activity = station.activity;
    act.textContent = ACTIVITY_TEXT[station.activity] ?? station.activity;
    const activityCell = cell(act);
    if (station.detail) {
      activityCell.append(document.createTextNode(` → ${station.detail}`));
    }
    if (station.connected && station.activity !== "connected") {
      activityCell.title = "a session with this station has been up from here";
    }
    cell(String(station.count), "num");
    const button = document.createElement("button");
    button.className = "small";
    button.textContent = "Call";
    button.title = `Put ${station.callsign} in the Call box on the Session tab`;
    button.addEventListener("click", () => {
      $("remote").value = station.callsign;
      selectTab($("tab-session"));
      $("remote").focus();
    });
    cell(button);
    body.append(row);
  }
}

function sortHeard(key) {
  if (heardSort.key === key) heardSort.direction = -heardSort.direction;
  else heardSort = { key, direction: key.endsWith("_ms") || key.endsWith("_db") || key === "count" ? -1 : 1 };
  for (const button of document.querySelectorAll("#heard-table button.sort")) {
    if (button.dataset.sort === key) {
      button.setAttribute("aria-sort", heardSort.direction < 0 ? "descending" : "ascending");
    } else {
      button.removeAttribute("aria-sort");
    }
  }
  renderHeard();
}

async function clearHeard() {
  if (!window.confirm("Forget every station heard? The modem's list is cleared too.")) return;
  try {
    const result = await call("heard.clear");
    heardList = [];
    renderHeard();
    $("heard-note").textContent = `Forgot ${result.cleared ?? 0}.`;
  } catch (error) {
    $("heard-note").textContent = error.message;
  }
}

// ── diagnostics: the constellation, the spectrum, the frames ────────

let spectrumTimer = null;
let spectrumBusy = false;
let palette = null;
let spectrumPolls = 0;

// The waterfall's controls, as any waterfall has them: the floor (auto follows the
// quietest fifth of the spectrum), the range above it, lines per second, the palette.
const waterfall = { auto: true, floor: -90, gain: 45, speed: 8, palette: "aether" };

const PALETTES = {
  // black, the accent, white: the panel's own
  aether: null,
  // the classic blue waterfall: black, blue, cyan, white
  blue: [[0, 0, 0], [0, 0, 110], [0, 70, 255], [0, 220, 255], [255, 255, 255]],
  turbo: [[48, 18, 59], [70, 107, 227], [26, 228, 182], [164, 252, 60], [251, 190, 26], [227, 72, 6], [122, 4, 3]],
  viridis: [[68, 1, 84], [59, 82, 139], [33, 145, 140], [94, 201, 98], [253, 231, 37]],
  grey: [[0, 0, 0], [255, 255, 255]],
};

function loadWaterfallSettings() {
  try {
    const kept = JSON.parse(localStorage.getItem("aether.waterfall") ?? "{}");
    for (const key of Object.keys(waterfall)) if (key in kept) waterfall[key] = kept[key];
  } catch {
    // nothing kept, or nothing readable: the defaults
  }
}

function saveWaterfallSettings() {
  try {
    localStorage.setItem("aether.waterfall", JSON.stringify(waterfall));
  } catch {
    // a browser that keeps nothing keeps nothing
  }
}

function showWaterfallSettings() {
  $("wf-auto").checked = waterfall.auto;
  $("wf-floor").value = String(waterfall.floor);
  $("wf-floor").disabled = waterfall.auto;
  $("wf-floor-value").textContent = waterfall.auto
    ? `auto ${Math.round(waterfallFloor)} dBFS`
    : `${waterfall.floor} dBFS`;
  $("wf-gain").value = String(waterfall.gain);
  $("wf-gain-value").textContent = `${waterfall.gain} dB`;
  $("wf-speed").value = String(waterfall.speed);
  $("wf-palette").value = waterfall.palette;
}

function wireWaterfallControls() {
  loadWaterfallSettings();
  $("wf-auto").addEventListener("change", () => {
    waterfall.auto = $("wf-auto").checked;
    if (!waterfall.auto) waterfall.floor = Math.round(waterfallFloor);
    saveWaterfallSettings();
    showWaterfallSettings();
  });
  $("wf-floor").addEventListener("input", () => {
    waterfall.floor = Number($("wf-floor").value);
    saveWaterfallSettings();
    showWaterfallSettings();
  });
  $("wf-gain").addEventListener("input", () => {
    waterfall.gain = Number($("wf-gain").value);
    saveWaterfallSettings();
    showWaterfallSettings();
  });
  $("wf-speed").addEventListener("change", () => {
    waterfall.speed = Number($("wf-speed").value);
    saveWaterfallSettings();
  });
  $("wf-palette").addEventListener("change", () => {
    waterfall.palette = $("wf-palette").value;
    palette = null;
    saveWaterfallSettings();
  });
  showWaterfallSettings();
}

function startScopes() {
  if (spectrumTimer !== null) return;
  spectrumTimer = setInterval(pollSpectrum, 125);
  pollSpectrum();
  fetchConstellation();
  renderFrames();
}

function stopScopes() {
  if (spectrumTimer === null) return;
  clearInterval(spectrumTimer);
  spectrumTimer = null;
}

function scopesWanted() {
  if (panelShown("diagnostics") && socket && socket.readyState === WebSocket.OPEN) startScopes();
  else stopScopes();
}

async function pollSpectrum() {
  if (spectrumBusy || !socket || socket.readyState !== WebSocket.OPEN) return;
  spectrumBusy = true;
  try {
    const spectrum = await call("spectrum");
    drawSpectrum(spectrum);
    // the spectrum refreshes eight times a second; the waterfall advances at its own pace
    spectrumPolls += 1;
    if (spectrumPolls % Math.max(1, Math.round(8 / waterfall.speed)) === 0) drawWaterfall(spectrum);
  } catch {
    // the next tick tries again; a missed frame of a scope is nothing
  } finally {
    spectrumBusy = false;
  }
}

const SPECTRUM_TOP_HZ = 4000;

function drawSpectrum(spectrum) {
  const { ctx, width, height } = surface("chart-spectrum", 600, 110);
  const c = tokens();
  ctx.clearRect(0, 0, width, height);
  const bins = spectrum.bins_db ?? [];
  const caption = $("spectrum-caption");
  if (bins.length === 0) {
    caption.textContent = "waiting for audio";
    return;
  }
  const binHz = spectrum.bin_hz;
  const low = -110;
  const high = 0;
  const x = (hz) => (hz / SPECTRUM_TOP_HZ) * width;
  const y = (db) => (1 - (Math.max(low, Math.min(high, db)) - low) / (high - low)) * (height - 12) + 2;
  // the modem's passband, marked
  const [lo, hi] = spectrum.passband_hz ?? [0, 0];
  ctx.fillStyle = c.accent;
  ctx.globalAlpha = 0.08;
  ctx.fillRect(x(lo), 0, x(hi) - x(lo), height);
  ctx.globalAlpha = 1;
  ctx.strokeStyle = c.grid;
  ctx.lineWidth = 1;
  ctx.font = `9px ${c.numerals}`;
  ctx.textBaseline = "top";
  ctx.textAlign = "center";
  for (let hz = 500; hz < SPECTRUM_TOP_HZ; hz += 500) {
    ctx.beginPath();
    ctx.moveTo(x(hz), 0);
    ctx.lineTo(x(hz), height - 12);
    ctx.stroke();
    ctx.fillStyle = c.ink;
    ctx.fillText(hz % 1000 === 0 ? `${hz / 1000} kHz` : String(hz), x(hz), height - 10);
  }
  for (const db of [-20, -40, -60, -80, -100]) {
    ctx.beginPath();
    ctx.moveTo(0, y(db));
    ctx.lineTo(width, y(db));
    ctx.stroke();
  }
  ctx.strokeStyle = spectrum.transmitting ? c.tx : c.accent;
  ctx.lineWidth = 1.25;
  ctx.beginPath();
  bins.forEach((db, index) => {
    const at = x(index * binHz);
    if (index === 0) ctx.moveTo(at, y(db));
    else ctx.lineTo(at, y(db));
  });
  ctx.stroke();
  let peak = 0;
  let peakDb = -Infinity;
  bins.forEach((db, index) => {
    if (db > peakDb) {
      peakDb = db;
      peak = index;
    }
  });
  caption.textContent = spectrum.transmitting
    ? "transmitting — this is the sound card's own output"
    : `peak ${Math.round(peak * binHz)} Hz at ${peakDb.toFixed(0)} dBFS`;
}

function heat(fraction) {
  if (!palette) {
    const c = tokens();
    const hex = (h) => {
      const m = /^#([0-9a-f]{6})$/i.exec(h);
      if (!m) return [64, 64, 64];
      const v = parseInt(m[1], 16);
      return [(v >> 16) & 255, (v >> 8) & 255, v & 255];
    };
    const stops = PALETTES[waterfall.palette] ?? [hex(c.plot), hex(c.accent), [255, 255, 255]];
    palette = [];
    for (let i = 0; i < 256; i++) {
      const p = (i / 255) * (stops.length - 1);
      const a = stops[Math.floor(p)];
      const b = stops[Math.min(stops.length - 1, Math.floor(p) + 1)];
      const f = p - Math.floor(p);
      palette.push([0, 1, 2].map((k) => Math.round(a[k] + (b[k] - a[k]) * f)));
    }
  }
  return palette[Math.max(0, Math.min(255, Math.round(fraction * 255)))];
}

let waterfallFloor = -90;

function drawWaterfall(spectrum) {
  const bins = spectrum.bins_db ?? [];
  if (bins.length === 0) return;
  const canvas = $("chart-waterfall");
  const width = Math.max(1, canvas.clientWidth || 600);
  const height = 110;
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  const ctx = canvas.getContext("2d");
  // the newest line at the top; everything else moves down one
  ctx.drawImage(canvas, 0, 0, width, height - 1, 0, 1, width, height - 1);
  // the floor follows the quietest fifth of the spectrum, slowly, so the noise stays dark
  // and a signal stays bright whatever the receive level — unless the operator set it
  const sorted = [...bins].sort((a, b) => a - b);
  const quiet = sorted[Math.floor(sorted.length / 5)];
  waterfallFloor += (quiet - waterfallFloor) * 0.1;
  if (waterfall.auto && spectrumPolls % 8 === 0) {
    $("wf-floor-value").textContent = `auto ${Math.round(waterfallFloor)} dBFS`;
  }
  const floor = waterfall.auto ? waterfallFloor : waterfall.floor;
  const span = waterfall.gain;
  const row = ctx.createImageData(width, 1);
  const binHz = spectrum.bin_hz;
  for (let px = 0; px < width; px++) {
    const hz = (px / width) * SPECTRUM_TOP_HZ;
    const index = Math.min(bins.length - 1, Math.round(hz / binHz));
    const [r, g, b] = heat((bins[index] - floor) / span);
    row.data[px * 4] = r;
    row.data[px * 4 + 1] = g;
    row.data[px * 4 + 2] = b;
    row.data[px * 4 + 3] = 255;
  }
  ctx.putImageData(row, 0, 0);
}

let constellationBusy = false;

async function fetchConstellation() {
  if (constellationBusy || !socket || socket.readyState !== WebSocket.OPEN) return;
  constellationBusy = true;
  try {
    drawConstellation(await call("constellation"));
  } catch {
    // nothing to draw is nothing to draw
  } finally {
    constellationBusy = false;
  }
}

function drawConstellation(result) {
  const canvas = $("chart-constellation");
  const size = canvas.clientWidth || 240;
  const { ctx, width, height } = surface("chart-constellation", size, size);
  const c = tokens();
  ctx.clearRect(0, 0, width, height);
  const points = result.points ?? [];
  const frame = result.frame;
  const caption = $("const-caption");
  const half = width / 2;
  ctx.strokeStyle = c.grid;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(half, 0);
  ctx.lineTo(half, height);
  ctx.moveTo(0, half);
  ctx.lineTo(width, half);
  ctx.stroke();
  if (points.length === 0 || !frame) {
    caption.textContent = "no frame yet";
    return;
  }
  let reach = 1.5;
  for (const [i, q] of points) reach = Math.max(reach, Math.abs(i) + 0.1, Math.abs(q) + 0.1);
  const scale = half / reach;
  ctx.beginPath();
  ctx.arc(half, half, scale, 0, Math.PI * 2);
  ctx.stroke();
  ctx.fillStyle = frame.decoded ? c.accent : c.error;
  ctx.globalAlpha = 0.75;
  for (const [i, q] of points) {
    ctx.fillRect(half + i * scale - 1.5, half - q * scale - 1.5, 3, 3);
  }
  ctx.globalAlpha = 1;
  const mode = modeTable[frame.mode];
  caption.textContent =
    `${frame.kind} · mode ${frame.mode}${mode ? ` ${mode.name}` : ""} · ` +
    `${Number(frame.snr_db).toFixed(1)} dB · ${frame.decoded ? "decoded" : "not decoded"}`;
}

function renderFrames() {
  $("frames-empty").hidden = framesSeen.length > 0;
  $("frames").parentElement.parentElement.hidden = framesSeen.length === 0;
  const body = $("frames");
  body.replaceChildren();
  for (const frame of framesSeen) {
    const row = document.createElement("tr");
    row.dataset.decoded = String(frame.decoded === true);
    const cell = (text, className) => {
      const td = document.createElement("td");
      if (className) td.className = className;
      td.textContent = text;
      row.append(td);
      return td;
    };
    cell(clock(frame.at), "numerals");
    cell(frame.kind);
    cell(frame.from || frame.to ? `${frame.from ?? "?"} → ${frame.to ?? (frame.from ? "me" : "?")}` : "—");
    cell(String(frame.mode), "num");
    cell(String(frame.rv), "num");
    cell(`${Number(frame.snr_db).toFixed(1)}`, "num");
    cell(typeof frame.cfo_hz === "number" ? `${frame.cfo_hz.toFixed(0)} Hz` : "—", "num");
    cell(Number(frame.confidence).toFixed(2), "num");
    cell(frame.decoded ? "yes" : "no", frame.decoded ? "" : "bad");
    cell(String(frame.bytes), "num");
    cell(frame.control ?? "");
    body.append(row);
  }
}

// ── compact ─────────────────────────────────────────────────────────

let sizeBeforeCompact = null;

// In the desktop shell the window follows the view: compact fits the state strip and
// the four readings, and coming back restores what the operator had. In a browser tab
// there is no window to size; the layout still compacts.
async function fitShellWindow(on) {
  const tauri = window.__TAURI__;
  const getWindow = tauri?.window?.getCurrentWindow;
  const LogicalSize = tauri?.dpi?.LogicalSize ?? tauri?.window?.LogicalSize;
  if (!getWindow || !LogicalSize) return;
  try {
    const win = getWindow();
    if (on) {
      const scale = await win.scaleFactor();
      const size = await win.innerSize();
      sizeBeforeCompact = { width: size.width / scale, height: size.height / scale };
      // let the compact layout settle before measuring what it needs
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const height = Math.ceil(document.body.getBoundingClientRect().height) + 4;
      await win.setMinSize(new LogicalSize(360, 120));
      await win.setSize(new LogicalSize(Math.min(sizeBeforeCompact.width, 520), height));
    } else if (sizeBeforeCompact) {
      await win.setMinSize(new LogicalSize(420, 300));
      await win.setSize(new LogicalSize(sizeBeforeCompact.width, sizeBeforeCompact.height));
      sizeBeforeCompact = null;
    }
  } catch {
    // the shell refused (an older shell without the panel's capability) or a browser
  }
}

function setCompact(on) {
  document.body.classList.toggle("compact", on);
  $("btn-compact").setAttribute("aria-pressed", String(on));
  if (on) selectTab($("tab-status"));
  fitShellWindow(on);
  try {
    localStorage.setItem("aether.compact", on ? "1" : "0");
  } catch {
    // a browser that keeps nothing keeps nothing
  }
  drawChart();
  drawStatusChart();
}


// Short enough for a pill; the long form is the pill's tooltip.
const COUNTER_LABELS = {
  frames_sent: ["Frames sent", "Data frames put on the air"],
  frames_resent: ["Retransmitted", "Of the frames sent, how many were retransmissions"],
  frames_received: ["Frames received", "Frames decoded from the other station"],
  frames_failed: ["Failed to decode", "Frames detected that did not decode"],
  harq_rescues: ["HARQ rescues", "Frames recovered by combining retransmissions"],
  bytes_delivered: ["Bytes delivered", "Payload bytes handed to the application"],
  bursts: ["Bursts", "Bursts sent"],
  turns: ["Turns", "Times the sending role changed hands"],
  ack_timeouts: ["ACKs missed", "Acknowledgements that never arrived in time"],
  transmissions: ["Transmissions", "Times the radio was keyed"],
  deferred_for_busy: ["Held for busy", "Transmissions held back for a busy channel"],
  watchdog_trips: ["Watchdog trips", "Times the key-time watchdog released the key"],
  probes_sent: ["Probes sent", "Probes this station sent"],
  probe_replies: ["Probes answered", "Of the probes sent, how many drew an answer"],
  probes_answered: ["Probes taken", "Probes from other stations this one answered"],
};

function renderCounters(counters) {
  const box = $("counters");
  box.replaceChildren();
  for (const [key, [label, long]] of Object.entries(COUNTER_LABELS)) {
    const value = counters[key];
    if (value === undefined) continue;
    const pill = document.createElement("div");
    pill.className = "pill";
    pill.title = long;
    const name = document.createElement("span");
    name.className = "k";
    name.textContent = label;
    const number = document.createElement("span");
    number.className = "v";
    number.textContent = String(value);
    if (key === "watchdog_trips" && value > 0) pill.dataset.state = "error";
    pill.append(name, number);
    box.append(pill);
  }
}

// ── the channel, last two minutes ───────────────────────────────────
//
// One reading every half second from the busy detector, drawn as a bar: its height is
// the received level, dim while only noise is arriving and bright while a burst is being
// received, and a short tick at the baseline where this station was transmitting and its
// receiver was muted. A faint dashed rule marks every change between the two, so the
// rhythm of a session — burst, acknowledgement, burst — is read off the top of the chart.

const history = [];

function drawChart() {
  const now = Date.now();
  if (history.length < 2) {
    surface("chart", 450, 180).ctx.clearRect(0, 0, 4096, 4096);
    return;
  }
  let low = Infinity;
  let high = -Infinity;
  for (const point of history) {
    low = Math.min(low, point.floor);
    high = Math.max(high, point.floor + 12, point.level ?? point.floor);
  }
  low = Math.floor((low - 3) / 5) * 5;
  high = Math.ceil((high + 3) / 5) * 5;
  const f = chartFrame("chart", 450, 180, low, high);
  const { ctx, c } = f;
  drawValueAxis(f, low, high, high - low > 40 ? 10 : 5);
  const x = drawTimeAxis(f, now, LEVEL_SPAN_MS, 30_000, 60_000);
  const slot = (500 / LEVEL_SPAN_MS) * f.plotWidth;
  const bar = Math.max(1, slot - 0.6);
  const base = f.bottom;

  for (const point of history) {
    const x0 = x(point.at) - slot;
    if (point.tx) {
      ctx.fillStyle = c.tx;
      ctx.fillRect(x0, base - 5, bar, 5);
    } else if (point.level !== null && point.level !== undefined) {
      ctx.fillStyle = point.rx ? c.rx : withAlpha(c.rx, 0.45);
      const top = Math.min(f.y(point.level), base - 1);
      ctx.fillRect(x0, top, bar, base - top);
    }
  }

  // a rule at every change between receiving and transmitting
  ctx.strokeStyle = withAlpha(c.ink, 0.3);
  ctx.lineWidth = 1;
  ctx.setLineDash([2, 3]);
  for (let i = 1; i < history.length; i++) {
    if (history[i].tx !== history[i - 1].tx) {
      const at = x(history[i].at) - slot;
      ctx.beginPath();
      ctx.moveTo(at, CHART_TOP);
      ctx.lineTo(at, base);
      ctx.stroke();
    }
  }
  ctx.setLineDash([]);

  // the noise floor the busy detector keeps, which the bars are read against
  ctx.strokeStyle = c.ink2;
  ctx.lineWidth = 1.25;
  ctx.setLineDash([4, 4]);
  ctx.beginPath();
  history.forEach((point, index) => {
    if (index === 0) ctx.moveTo(x(point.at), f.y(point.floor));
    else ctx.lineTo(x(point.at), f.y(point.floor));
  });
  ctx.stroke();
  ctx.setLineDash([]);

  drawLegend(f, [
    [c.ink2, "level, dBFS"],
    [c.rx, "▍ receiving"],
    [c.tx, "▍ transmitting"],
    [c.ink2, "- - noise floor"],
  ]);
}

window.addEventListener("resize", () => {
  drawChart();
  drawStatusChart();
});
document.addEventListener("visibilitychange", scopesWanted);

// ── capabilities and devices ────────────────────────────────────────

/// The fastest-mode list, from the mode table the modem reports. The table is the
/// running waveform's; when the operator picks the other bandwidth the list shrinks or
/// grows to that table's size (ten modes at 500 Hz, fourteen at 2300) and the modem
/// reports the real names once it has restarted into it.
function fillModes() {
  const select = $("radio-max-mode");
  if (select.options.length === modeTable.length && modeTable.length > 0) return;
  const before = select.value;
  select.replaceChildren();
  for (const mode of modeTable) {
    const option = document.createElement("option");
    option.value = String(mode.index);
    option.textContent = `${mode.index} — ${mode.name}`;
    select.append(option);
  }
  if (before) select.value = before;
}

async function loadCapabilities() {
  let caps;
  try {
    caps = await call("capabilities");
  } catch {
    return;
  }
  modeTable = caps.modes ?? [];
  fillModes();
  const usable = new Set(caps.usable_modes ?? []);
  const body = $("modes");
  body.replaceChildren();
  for (const mode of modeTable) {
    const row = document.createElement("tr");
    row.dataset.usable = String(usable.has(mode.index));
    for (const [text, numeric] of [
      [String(mode.index), true],
      // a floor mode rides the long floor frame (ADR-0009): its bytes are per 4.2 s
      [mode.floor ? `${mode.name} · floor` : mode.name, false],
      [String(mode.payload_bytes), true],
      [`${Math.round(mode.net_bit_rate)} bit/s`, true],
      [`${mode.threshold_db.toFixed(1)} dB`, true],
    ]) {
      const cell = document.createElement("td");
      if (numeric) cell.className = "num";
      cell.textContent = text;
      row.append(cell);
    }
    body.append(row);
  }
}

async function loadDevices() {
  let devices;
  try {
    devices = await call("devices.list");
  } catch {
    return;
  }
  const fill = (select, entries, placeholder) => {
    select.replaceChildren();
    const none = document.createElement("option");
    none.value = "";
    none.textContent = placeholder;
    select.append(none);
    for (const entry of entries) {
      const option = document.createElement("option");
      option.value = entry.name;
      option.textContent = entry.label ?? entry.name;
      select.append(option);
    }
    select.addEventListener("change", writeConfig);
  };
  devicesSeen = {
    devices: devices.devices ?? [],
    serial_ports: devices.serial_ports ?? [],
    gpio_interfaces: devices.gpio_interfaces ?? [],
  };
  const all = devicesSeen.devices;
  fill($("dev-in"), all.filter((d) => d.input), "system default");
  fill($("dev-out"), all.filter((d) => d.output), "system default");
  // a radio's USB port is often two serial ports and only one of them keys; the driver's
  // description is how an operator tells them apart, so it goes next to the name
  fill(
    $("dev-ptt"),
    [
      ...devicesSeen.serial_ports.map((p) => ({
        name: p.name,
        label: p.description ? `${p.name} — ${p.description}` : p.name,
      })),
      // a CM108-class interface keys through its own codec: no serial port at all
      ...devicesSeen.gpio_interfaces.map((i) => ({
        name: `gpio:${i.path}`,
        label: `${i.name} — keys by GPIO`,
      })),
      { name: "rigctld", label: "rigctld — Hamlib rig control over the network" },
    ],
    "none (VOX or receive only)",
  );
  $("dev-ptt").addEventListener("change", showKeyingFields);
  $("ptt-line").addEventListener("change", showKeyingFields);
  $("ptt-gpio").addEventListener("change", writeConfig);
  $("ptt-protocol").addEventListener("change", showKeyingFields);
  showKeyingFields();
  $("dev-in").addEventListener("change", checkRates);
  $("dev-out").addEventListener("change", checkRates);
  fillProfiles();
  checkRates();
  writeConfig();
}

// What the daemon is actually running, as `config.get` reported it.
let liveConfig = null;
// The setup section's own words, to put back once a failed load has been followed by a
// good one — the panel reconnects after a restart, and "the connection closed" would
// otherwise stay on screen with nothing closed.
let setupNoteAtRest = null;
let liveKeys = [];
let configPath = "";

async function loadConfig() {
  let answer;
  try {
    answer = await call("config.get");
  } catch (error) {
    // A daemon started without a configuration file says so rather than pretending; the
    // panel then shows what it would write instead of what it would change.
    $("setup-note").textContent = error.message;
    $("wz-save").disabled = true;
    writeConfig();
    return;
  }
  liveConfig = answer.config ?? {};
  liveKeys = answer.live_keys ?? [];
  configPath = answer.path ?? "";
  $("help-config").textContent = configPath || "—";
  $("help-dir").textContent = configPath
    ? configPath.replace(/[\\/][^\\/]*$/, "")
    : "—";
  $("wz-save").disabled = false;
  setupNoteAtRest ??= $("setup-note").textContent;
  $("setup-note").textContent = setupNoteAtRest;
  showTxLevel(liveConfig.audio?.tx_level ?? 0.25);

  // show the operator what is there now, so the form is not a blank slate over live settings
  if (!$("wz-call").value && liveConfig.callsign && liveConfig.callsign !== "N0CALL") {
    $("wz-call").value = liveConfig.callsign;
    markStep(1, true);
  }
  const operator = liveConfig.operator ?? {};
  $("op-grid").value = operator.grid ?? "";
  $("op-rig").value = operator.rig ?? "";
  $("op-power").value = operator.power_w == null ? "" : String(operator.power_w);
  $("op-antenna").value = operator.antenna ?? "";
  select($("dev-in"), liveConfig.audio?.input ?? "");
  select($("dev-out"), liveConfig.audio?.output ?? "");
  const ptt = liveConfig.ptt ?? {};
  if (ptt.kind === "cm108") {
    // the file may name the interface or leave it to the first one found
    const listed = [...$("dev-ptt").options].map((o) => o.value).filter((v) => v.startsWith("gpio:"));
    const wanted = `gpio:${ptt.device ?? ""}`;
    select($("dev-ptt"), listed.includes(wanted) ? wanted : (listed[0] ?? ""));
    select($("ptt-gpio"), String(ptt.gpio ?? 3));
  } else {
    select($("dev-ptt"), ptt.kind === "rigctld" ? "rigctld" : (ptt.port ?? ""));
  }
  select($("ptt-line"), ptt.kind === "cat" ? "cat" : (ptt.line ?? "rts"));
  if (ptt.address) $("ptt-address").value = ptt.address;
  if (ptt.kind === "cat") {
    select($("ptt-protocol"), ptt.protocol ?? "yaesu");
    $("ptt-baud").value = String(ptt.baud ?? 38400);
    if (ptt.civ_address != null) $("ptt-civ").value = ptt.civ_address.toString(16).toUpperCase();
  }
  showKeyingFields();
  const radio = liveConfig.radio ?? {};
  fillModes();
  select($("radio-bandwidth"), String(radio.bandwidth ?? 2300));
  $("radio-answer-only").checked = radio.answer_only === true;
  select($("radio-max-mode"), String(radio.max_mode ?? 13));
  $("radio-compress").checked = radio.compress === true;
  $("radio-wait").checked = radio.wait_for_clear !== false;
  $("radio-busy-db").value = String(radio.busy_threshold_db ?? 6);
  $("radio-max-key").value = String(radio.max_key_s ?? 30);
  $("radio-cwid").checked = radio.cw_id === true;
  $("radio-cwid-interval").value = String(radio.cw_id_interval_s ?? 600);
  $("radio-cwid-wpm").value = String(radio.cw_id_wpm ?? 20);
  // the operator's saved Interface choice wins over a guess; without one, the file's
  // capture device says which interface this station is, better than whatever else is
  // plugged in. Either way a choice already made by hand this session is left alone.
  const savedProfile = liveConfig.panel?.interface;
  const savedIndex = savedProfile
    ? PROFILES.findIndex((p) => p.id === savedProfile)
    : -1;
  if (!profileChosen && $("wz-profile").options.length > 0) {
    const index = savedIndex >= 0 ? savedIndex : guessProfile();
    $("wz-profile").value = String(index);
    $("wz-profile-note").textContent = PROFILES[index]?.note ?? "";
    // a stored choice is authoritative: hold it through the next device rebuild
    if (savedIndex >= 0) profileChosen = true;
  }
  select($("update-channel"), liveConfig.update?.channel ?? "stable");
  $("update-check").checked = liveConfig.update?.check !== false;
  $("record-auto").checked = liveConfig.record?.auto === true;
  $("record-standing").value = liveConfig.record?.notes ?? "";
  $("host-enabled").checked = liveConfig.host?.enabled === true;
  $("host-port").value = String(portOf(liveConfig.host?.bind) ?? 8300);
  // the file's devices, not the profile's guess, are what the warning should be about
  checkRates();
  checkModemSettings();
  checkAppSettings();
  writeConfig();
}

function select(element, value) {
  if ([...element.options].some((option) => option.value === value)) element.value = value;
}

// The modem runs at 48 kHz and does not resample. On Windows a USB radio codec runs at
// whatever Sound settings say, and 44.1 kHz is a common factory setting — so say so here,
// where it can be fixed, rather than let the daemon refuse to start later.
function rateProblem(name, direction) {
  if (!name) return "";
  // Windows lists a USB codec twice under one name, once as a capture endpoint and once
  // as playback, so the entry that matters is the one facing the right way
  const capture = direction === "capture";
  const device = devicesSeen.devices.find(
    (d) => d.name === name && (capture ? d.input : d.output),
  );
  const rates = device?.[capture ? "input_rates" : "output_rates"] ?? [];
  if (rates.length === 0 || rates.includes(48000)) return "";
  return `${name} ${direction === "capture" ? "captures" : "plays"} at ${rates.join(" or ")} Hz, and the modem needs 48000 Hz. On Windows: Settings > System > Sound > the device > Advanced, set the format to 48000 Hz, then reload this page.`;
}

function checkRates() {
  const problems = [
    rateProblem($("dev-in").value, "capture"),
    rateProblem($("dev-out").value, "playback"),
  ].filter(Boolean);
  $("rate-note").textContent = problems.join(" ");
  $("rate-note").hidden = problems.length === 0;
  markStep(2, problems.length === 0 && Boolean($("wz-profile").value));
  return problems.length === 0;
}

function writeConfig() {
  $("config-out").textContent = liveConfig
    ? JSON.stringify(liveConfig, null, 2)
    : "The daemon has no configuration file to change.";
  $("footer-config").textContent = configPath;
}

/// The line, address and CAT fields belong to one keying method each; show what applies.
function showKeyingFields() {
  const chosen = $("dev-ptt").value;
  const gpio = chosen.startsWith("gpio:");
  const onPort = chosen !== "" && chosen !== "rigctld" && !gpio;
  $("ptt-line").hidden = !onPort;
  $("ptt-gpio").hidden = !gpio;
  $("ptt-address").hidden = chosen !== "rigctld";
  const cat = onPort && $("ptt-line").value === "cat";
  $("cat-row").hidden = !cat;
  $("civ-fields").hidden = !cat || $("ptt-protocol").value !== "icom";
}

/// The keying settings as the file takes them: one kind, and only that kind's fields.
function keyingChanges() {
  const chosen = $("dev-ptt").value;
  if (chosen === "") return { "ptt.kind": "none" };
  if (chosen === "rigctld") {
    return {
      "ptt.kind": "rigctld",
      "ptt.address": $("ptt-address").value.trim() || "127.0.0.1:4532",
    };
  }
  if (chosen.startsWith("gpio:")) {
    return {
      "ptt.kind": "cm108",
      "ptt.device": chosen.slice(5),
      "ptt.gpio": Number($("ptt-gpio").value) || 3,
    };
  }
  if ($("ptt-line").value === "cat") {
    const protocol = $("ptt-protocol").value;
    const changes = {
      "ptt.kind": "cat",
      "ptt.port": chosen,
      "ptt.protocol": protocol,
      "ptt.baud": numberIn("ptt-baud") ?? 38400,
      "ptt.source": "data",
    };
    if (protocol === "icom") {
      const address = parseInt($("ptt-civ").value.trim().replace(/^0x/i, ""), 16);
      if (Number.isInteger(address)) changes["ptt.civ_address"] = address;
    }
    return changes;
  }
  return { "ptt.kind": "serial", "ptt.port": chosen, "ptt.line": $("ptt-line").value };
}

/// A number typed into a field, or nothing when it is not one.
function numberIn(id) {
  const value = Number($(id).value);
  return Number.isFinite(value) ? value : null;
}

/// The port of a `host:port` address, or nothing.
function portOf(address) {
  const port = Number(String(address ?? "").split(":").pop());
  return Number.isInteger(port) && port > 0 ? port : null;
}

/// Everything the form would change, as the dotted keys `config.set` takes.
function formChanges() {
  const changes = {
    "audio.input": $("dev-in").value || null,
    "audio.output": $("dev-out").value || null,
    ...keyingChanges(),
  };
  const callsign = $("wz-call").value.trim().toUpperCase();
  if (callsign) changes.callsign = callsign;
  // who and where, for the field log: optional, an empty field is "not said"
  changes["operator.grid"] = $("op-grid").value.trim().toUpperCase();
  changes["operator.rig"] = $("op-rig").value.trim();
  changes["operator.power_w"] = numberIn("op-power");
  changes["operator.antenna"] = $("op-antenna").value.trim();
  changes["radio.bandwidth"] = Number($("radio-bandwidth").value);
  changes["radio.answer_only"] = $("radio-answer-only").checked;
  changes["radio.max_mode"] = Number($("radio-max-mode").value);
  changes["radio.compress"] = $("radio-compress").checked;
  changes["radio.wait_for_clear"] = $("radio-wait").checked;
  for (const [id, key] of [
    ["radio-busy-db", "radio.busy_threshold_db"],
    ["radio-max-key", "radio.max_key_s"],
    ["radio-cwid-interval", "radio.cw_id_interval_s"],
    ["radio-cwid-wpm", "radio.cw_id_wpm"],
  ]) {
    const value = numberIn(id);
    if (value !== null) changes[key] = value;
  }
  changes["radio.cw_id"] = $("radio-cwid").checked;
  changes["update.channel"] = $("update-channel").value;
  changes["update.check"] = $("update-check").checked;
  changes["record.auto"] = $("record-auto").checked;
  changes["record.notes"] = $("record-standing").value.trim();
  changes["audio.tx_level"] = txLevel();
  changes["host.enabled"] = $("host-enabled").checked;
  // the Interface is a panel choice, not a modem setting; it is kept so the dropdown
  // comes back to what the operator picked instead of being re-guessed from the devices
  const chosen = PROFILES[Number($("wz-profile").value)];
  changes["panel.interface"] = chosen?.id ?? null;
  const hostPort = Number($("host-port").value);
  if (Number.isInteger(hostPort) && hostPort > 0 && hostPort < 65535) {
    changes["host.bind"] = `127.0.0.1:${hostPort}`;
  }
  return changes;
}

/// What a save's answer means for the operator, having done the restart when there is one
/// to do and somebody to do it.
///
/// A sound card, a serial port and the callsign are taken on when the modem starts, so a
/// change to one is on the file and not in the modem until it starts again. Under the
/// desktop shell (or systemd) the daemon can ask to be started again and it happens in a
/// few seconds, with the panel reconnecting by itself; from a terminal there is nobody to
/// do it, and the note says so instead of pretending.
async function applied(answer) {
  const restart = answer.restart_required ?? [];
  if (restart.length === 0) return "In effect now.";
  if (lastStatus?.supervised !== true) {
    return `Restart the daemon for: ${restart.join(", ")}.`;
  }
  try {
    await call("shutdown", { restart: true });
  } catch (error) {
    return `Restart the daemon for: ${restart.join(", ")} (it would not restart itself: ${error.message}).`;
  }
  log(`the modem is restarting to apply ${restart.join(", ")}`);
  return `The modem is restarting to apply: ${restart.join(", ")}.`;
}

// ── the setup wizard ────────────────────────────────────────────────

// Known radio interfaces, by the device names they present. A profile only pre-fills the
// form; nothing here is authoritative, and every entry can be changed afterwards.
const PROFILES = [
  {
    id: "digirig",
    name: "Digirig",
    match: /USB PnP Sound Device|Digirig/i,
    ptt: "serial",
    line: "rts",
    note: "Digirig keys on RTS of its own serial port.",
  },
  {
    id: "icom-usb",
    name: "Icom with USB audio (IC-7300, IC-7610, IC-9700, IC-705)",
    match: /USB Audio CODEC/i,
    ptt: "serial",
    line: "rts",
    note: "Icom's USB port carries audio and a serial port; RTS keying needs 'USB SEND' set to RTS in the rig's menu.",
  },
  {
    id: "yaesu-usb",
    name: "Yaesu with USB audio (FT-991A, FTDX10, FT-710)",
    match: /USB AUDIO\s+CODEC/i,
    ptt: "serial",
    line: "rts",
    // the CP2105 bridge's two ports: the Enhanced one is CAT, the Standard one keys
    port: /Standard COM Port/i,
    note: "Yaesu's USB port is two serial ports: the Standard one keys on RTS (chosen here when it can be told apart; the rig's PTT select for the mode must be RTS), or pick the Enhanced one with 'CAT command' to key over CAT and record the frequency.",
  },
  {
    id: "cm108",
    name: "DRA, URI, RA-40 or another CM108 interface (keys by GPIO)",
    match: /C-Media|USB PnP Sound Device|USB Audio Device/i,
    ptt: "gpio",
    line: "rts",
    note: "The interface's codec carries the audio and keys the radio through its GPIO pin (3 on the DRA and URI boards), so there is no serial port to choose.",
  },
  {
    id: "signalink",
    name: "SignaLink USB",
    match: /USB Audio Device|SignaLink/i,
    ptt: "none",
    line: "rts",
    note: "SignaLink keys itself from the audio (VOX), so no keying line is needed.",
  },
  {
    id: "manual",
    name: "Manual — pick the modem devices below yourself",
    match: null,
    ptt: "serial",
    line: "rts",
    note: "Nothing is filled in for you: choose Capture, Playback and Keying under Modem devices below.",
  },
];

let devicesSeen = { devices: [], serial_ports: [], gpio_interfaces: [] };

// Whether the operator has picked an interface themselves. The list is rebuilt whenever
// the machine's devices are listed again — a device event, a reconnect — and a rebuild
// that guessed afresh each time snapped a chosen Yaesu back to the Icom that was also
// plugged in.
let profileChosen = false;

function fillProfiles() {
  const select = $("wz-profile");
  const before = select.value;
  select.replaceChildren();
  for (const [index, profile] of PROFILES.entries()) {
    const option = document.createElement("option");
    option.value = String(index);
    option.textContent = profile.name;
    select.append(option);
  }
  if (profileChosen && before !== "") {
    select.value = before;
    $("wz-profile-note").textContent = PROFILES[Number(before)]?.note ?? "";
    return;
  }
  select.value = String(guessProfile());
  applyProfile();
}

/// The interface to pre-select: the one the configured capture device belongs to when the
/// file names one, else the first whose devices are present, else "something else".
function guessProfile() {
  const configured = liveConfig?.audio?.input;
  if (configured) {
    const owner = PROFILES.findIndex((p) => p.match && p.match.test(configured));
    if (owner >= 0) return owner;
  }
  // a codec with a keying pin says what the interface is better than its audio name,
  // which a Digirig and a DRA share
  if (devicesSeen.gpio_interfaces.length > 0) {
    const gpio = PROFILES.findIndex((p) => p.ptt === "gpio");
    if (gpio >= 0) return gpio;
  }
  const present = PROFILES.findIndex(
    (p) => p.match && devicesSeen.devices.some((d) => p.match.test(d.name)),
  );
  return present >= 0 ? present : PROFILES.length - 1;
}

function applyProfile() {
  const profile = PROFILES[Number($("wz-profile").value)] ?? PROFILES.at(-1);
  $("wz-profile-note").textContent = profile.note;
  if (!profile.match) return;
  const matching = devicesSeen.devices.filter((d) => profile.match.test(d.name));
  const input = matching.find((d) => d.input)?.name;
  const output = matching.find((d) => d.output)?.name;
  if (input) select($("dev-in"), input);
  if (output) select($("dev-out"), output);
  checkRates();
  if (profile.ptt === "none") {
    $("dev-ptt").value = "";
  } else if (profile.ptt === "gpio") {
    const first = devicesSeen.gpio_interfaces[0];
    if (first) $("dev-ptt").value = `gpio:${first.path}`;
  } else if (profile.port && $("dev-ptt").value === "") {
    // the profile knows which of the interface's ports keys, by what the driver calls it
    const keying = devicesSeen.serial_ports.find((p) => profile.port.test(p.description));
    if (keying) $("dev-ptt").value = keying.name;
  } else if ($("dev-ptt").value === "" && devicesSeen.serial_ports.length === 1) {
    // one serial port on the machine: almost certainly the interface's
    $("dev-ptt").value = devicesSeen.serial_ports[0].name;
  }
  writeConfig();
}

function updateMeter(reading) {
  const meter = $("wz-meter");
  const fill = $("wz-meter-fill");
  const peak = $("wz-meter-peak");
  const percent = (db) => Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  if (!reading || reading.settled !== true) {
    fill.style.width = "0%";
    peak.style.left = "0%";
    meter.dataset.state = "quiet";
    meter.setAttribute("aria-valuenow", "-60");
    $("wz-level-reading").textContent = "Still listening.";
    return;
  }
  fill.style.width = `${percent(reading.rms_dbfs)}%`;
  peak.style.left = `${percent(reading.peak_dbfs)}%`;
  meter.setAttribute("aria-valuenow", reading.rms_dbfs.toFixed(0));
  const advice = reading.advice ?? "";
  meter.dataset.state = advice.startsWith("Clipping")
    ? "hot"
    : advice.startsWith("Almost")
      ? "warm"
      : advice.startsWith("Very quiet")
        ? "quiet"
        : "good";
  $("wz-level-reading").textContent =
    `${reading.rms_dbfs.toFixed(0)} dBFS RMS, peak ${reading.peak_dbfs.toFixed(0)} — ${advice}`;
  markStep(3, advice === "Good.");
}

function markStep(number, done) {
  const step = document.querySelector(`.step[data-step="${number}"]`);
  if (step) step.dataset.done = String(done);
}

function numberWithin(id, low, high) {
  const value = numberIn(id);
  return $(id).value.trim() !== "" && value !== null && value >= low && value <= high;
}

// Steps 4 and 5 hold defaults that are already in order, so their marks say "nothing
// here is wrong" rather than "you did this": a busy threshold, a key limit and a Morse
// schedule that parse, a host port a program can reach.
function checkModemSettings() {
  const morse = !$("radio-cwid").checked
    || (numberWithin("radio-cwid-interval", 10, 3600) && numberWithin("radio-cwid-wpm", 5, 40));
  const ok = numberWithin("radio-busy-db", 0, 60) && numberWithin("radio-max-key", 1, 600) && morse;
  markStep(4, ok);
  return ok;
}

function checkAppSettings() {
  const ok = numberWithin("host-port", 1024, 65535) && Boolean($("update-channel").value);
  markStep(5, ok);
  return ok;
}

async function wizardSave() {
  $("wz-save-note").textContent = "";
  const call_ = $("wz-call").value.trim().toUpperCase();
  if (!call_) {
    $("wz-save-note").textContent = "A callsign is required.";
    return;
  }
  const profile = PROFILES[Number($("wz-profile").value)] ?? PROFILES.at(-1);
  const changes = formChanges();
  if (profile.ptt === "none") {
    changes["ptt.kind"] = "none";
    for (const key of Object.keys(changes)) {
      if (key.startsWith("ptt.") && key !== "ptt.kind") delete changes[key];
    }
  }
  try {
    const answer = await call("config.set", changes);
    let note = `Saved. ${await applied(answer)}`;
    if (profile.ptt === "serial" && $("dev-ptt").value === "") {
      // the profile wants a keying line and none was chosen: the station is saved
      // receive-only, which is safe, but the operator should know why the radio
      // will not key
      note += " No keying port is chosen, so the radio will not key: pick the interface's serial port under Keying below and save again.";
    }
    $("wz-save-note").textContent = note;
    markStep(6, true);
    log(`settings saved (${(answer.changed ?? []).join(", ")})`);
    loadConfig();
    refreshStatus();
  } catch (error) {
    $("wz-save-note").textContent = error.message;
    log(error.message, true);
  }
}

async function transmitTest(method, seconds, label) {
  $("wz-tx-note").textContent = `${label}…`;
  try {
    await call(method, { duration_s: seconds });
    $("wz-tx-note").textContent = `${label} — watch the radio.`;
    log(label);
    return true;
  } catch (error) {
    $("wz-tx-note").textContent = error.message;
    log(error.message, true);
    return false;
  }
}

// ── the transmit level ──────────────────────────────────────────────
//
// The slider is in decibels below full scale, because that is how drive is thought about;
// the file holds the amplitude, because that is what the modem multiplies by. The level is
// live in the modem and applied as audio leaves, so moving it during a tune tone moves the
// tone — the rig's ALC answers at once.

const TUNE_SECONDS = 10;
let tuneTimer = null;

// The exact level the slider was last set from, so a save that did not touch the slider
// sends the file's own number back rather than a rounded cousin of it.
let txLevelShown = { level: 0.25, db: -12 };

function txLevel() {
  const db = Number($("tx-level").value);
  if (db === txLevelShown.db) return txLevelShown.level;
  return Number(10 ** (db / 20).toFixed(4));
}

function showTxLevel(level) {
  const db = Math.max(-34, Math.min(0, Math.round(20 * Math.log10(Math.max(level, 1e-3)))));
  txLevelShown = { level, db };
  $("tx-level").value = String(db);
  $("tx-level-reading").textContent = `${db === 0 ? "" : "−"}${Math.abs(db)} dB`;
}

async function saveTxLevel() {
  try {
    await call("config.set", { "audio.tx_level": txLevel() });
    log(`transmit level ${$("tx-level-reading").textContent}`);
  } catch (error) {
    $("wz-tx-note").textContent = error.message;
    log(error.message, true);
  }
}

function tuneButton(playing) {
  $("wz-tune").textContent = playing ? "Stop the tone" : `Tune tone, ${TUNE_SECONDS} s`;
  $("wz-tune").setAttribute("aria-pressed", String(playing));
  clearTimeout(tuneTimer);
  tuneTimer = playing ? setTimeout(() => tuneButton(false), TUNE_SECONDS * 1000 + 500) : null;
}

async function toggleTune() {
  if ($("wz-tune").getAttribute("aria-pressed") === "true") {
    tuneButton(false);
    try {
      await call("tune", { duration_s: 0 });
      $("wz-tx-note").textContent = "Tone stopped.";
    } catch (error) {
      $("wz-tx-note").textContent = error.message;
    }
    return;
  }
  const started = await transmitTest("tune", TUNE_SECONDS, `Tune tone for ${TUNE_SECONDS} s`);
  if (started) tuneButton(true);
}

// ── actions ─────────────────────────────────────────────────────────

function selectTab(tab, focus = false) {
  for (const other of document.querySelectorAll(".tab")) {
    const selected = other === tab;
    other.setAttribute("aria-selected", String(selected));
    // one tab stop for the whole list; the arrow keys move within it
    other.tabIndex = selected ? 0 : -1;
    $(`panel-${other.dataset.panel}`).hidden = !selected;
  }
  if (focus) tab.focus();
  if (tab.dataset.panel === "status") {
    drawChart();
    drawStatusChart();
  }
  if (tab.dataset.panel === "stations") renderHeard();
  scopesWanted();
}

function wire() {
  const tabs = [...document.querySelectorAll(".tab")];
  for (const [index, tab] of tabs.entries()) {
    tab.addEventListener("click", () => selectTab(tab));
    tab.addEventListener("keydown", (event) => {
      const step = { ArrowRight: 1, ArrowLeft: -1, Home: -index, End: tabs.length - 1 - index }[
        event.key
      ];
      if (step === undefined) return;
      event.preventDefault();
      selectTab(tabs[(index + step + tabs.length) % tabs.length], true);
    });
  }
  $("remote").addEventListener("keydown", (event) => {
    if (event.key === "Enter") $("btn-connect").click();
  });

  $("btn-connect").addEventListener("click", async () => {
    const remote = $("remote").value.trim().toUpperCase();
    if (!remote) return;
    await act(() => call("connect", { remote }), `calling ${remote}`);
  });
  $("btn-disconnect").addEventListener("click", () =>
    act(() => call("disconnect"), "closing the session"),
  );
  $("btn-abort").addEventListener("click", () =>
    act(() => call("abort"), "dropping the session"),
  );
  $("btn-beacon").addEventListener("click", () =>
    act(() => call("beacon"), "beaconing"),
  );
  $("btn-probe").addEventListener("click", async () => {
    const remote = $("remote").value.trim().toUpperCase();
    if (!remote) {
      $("remote").focus();
      return;
    }
    $("probe-result").textContent = `Probing ${remote}…`;
    delete $("probe-result").dataset.state;
    const ok = await act(() => call("probe", { remote }), `probing ${remote}`);
    if (!ok) $("probe-result").textContent = "";
  });
  $("btn-test").addEventListener("click", async () => {
    if (testRunning) {
      await act(() => call("test.abort"), "stopping the test session");
      return;
    }
    const remote = $("remote").value.trim().toUpperCase();
    if (!remote) {
      $("remote").focus();
      return;
    }
    const sure = window.confirm(
      `Run a test session with ${remote}? It sends a probe, a message, a file and a short burst at every mode — about five minutes of transmitting, ten at most, all recorded. Stop test ends it at any time.`,
    );
    if (!sure) return;
    $("test-result").textContent = `Test session with ${remote}: starting…`;
    delete $("test-result").dataset.state;
    const ok = await act(() => call("test.start", { remote }), `test session with ${remote}`);
    if (!ok) $("test-result").textContent = "";
  });
  $("btn-tune-to").addEventListener("click", tuneToMemory);
  $("btn-memory-add").addEventListener("click", openMemoryForm);
  $("btn-memory-remove").addEventListener("click", removeMemory);
  $("btn-memory-save").addEventListener("click", saveMemoryForm);
  $("btn-memory-cancel").addEventListener("click", closeMemoryForm);
  for (const id of ["memory-mhz", "memory-name"]) {
    $(id).addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        saveMemoryForm();
      } else if (event.key === "Escape") {
        closeMemoryForm();
      }
    });
  }
  $("btn-record").addEventListener("click", toggleRecording);
  $("btn-open-recordings").addEventListener("click", openRecordingsFolder);
  $("btn-copy-recordings").addEventListener("click", () => copyRecordingsPath(false));
  $("btn-copy-received").addEventListener("click", copyReceived);
  $("btn-reset-counters").addEventListener("click", async () => {
    await act(() => call("counters.reset"), "counters reset");
  });
  $("chart-tab-snr").addEventListener("click", () => selectStatusChart("snr"));
  $("chart-tab-speed").addEventListener("click", () => selectStatusChart("speed"));
  try {
    selectStatusChart(localStorage.getItem("aether.statuschart") || "snr");
  } catch {
    selectStatusChart("snr");
  }
  // notes typed before an automatic recording starts go with it
  $("record-notes").addEventListener("change", () => {
    const notes = $("record-notes").value.trim();
    call("record.notes", { notes: notes || null }).catch(() => {});
  });
  $("btn-send").addEventListener("click", async () => {
    const text = $("outgoing").value;
    if (!text) return;
    const ok = await act(
      () => call("send", { data: toBase64(text) }),
      `queued ${text.length} bytes`,
    );
    if (ok) $("outgoing").value = "";
  });
  $("btn-clear-log").addEventListener("click", () => $("log").replaceChildren());
  $("btn-heard-clear").addEventListener("click", clearHeard);
  for (const button of document.querySelectorAll("#heard-table button.sort")) {
    button.addEventListener("click", () => sortHeard(button.dataset.sort));
  }
  $("radio-bandwidth").addEventListener("change", () => {
    // the narrow table has thirteen modes: a fastest mode past it would be refused on save
    const modes = $("radio-bandwidth").value === "500" ? 13 : 14;
    const fastest = $("radio-max-mode");
    for (const option of fastest.options) option.hidden = Number(option.value) >= modes;
    if (Number(fastest.value) >= modes) fastest.value = String(modes - 1);
  });
  $("btn-compact").addEventListener("click", () => {
    setCompact(!document.body.classList.contains("compact"));
  });
  let compact = false;
  try {
    compact = localStorage.getItem("aether.compact") === "1";
  } catch {
    // nothing kept
  }
  if (compact) setCompact(true);
  // the stations' "3 min ago" and the dial move on their own
  setInterval(() => {
    if (panelShown("stations")) renderHeard();
    if (socket && socket.readyState === WebSocket.OPEN) refreshStatus();
  }, 15000);
  $("btn-diagnostics").addEventListener("click", copyDiagnostics);
  $("btn-contribute").addEventListener("click", contributeTestSession);
  $("wz-profile").addEventListener("change", () => {
    profileChosen = true;
    applyProfile();
  });
  for (const id of ["radio-busy-db", "radio-max-key", "radio-cwid", "radio-cwid-interval", "radio-cwid-wpm"]) {
    $(id).addEventListener("input", checkModemSettings);
  }
  for (const id of ["host-port", "update-channel", "host-enabled"]) {
    $(id).addEventListener("input", checkAppSettings);
  }
  wireWaterfallControls();
  loadHistory();
  renderFrames(); // what was kept shows before the first new frame does
  drawStatusChart(); // the axes are there from the start, frames or none
  $("wz-call").addEventListener("input", () => {
    const value = $("wz-call").value.trim().toUpperCase();
    const plausible = /^[A-Z0-9\/-]{1,9}$/.test(value);
    markStep(1, plausible);
    $("wz-call-note").textContent = plausible
      ? `Will go on the air as ${value}.`
      : "Letters, digits, - and /; up to nine characters.";
    writeConfig();
  });
  $("wz-ptt").addEventListener("click", () => transmitTest("ptt.test", 1.0, "Keyed for 1 s"));
  $("wz-tune").addEventListener("click", toggleTune);
  $("tx-level").addEventListener("input", () => showTxLevel(txLevel()));
  $("tx-level").addEventListener("change", saveTxLevel);
  $("wz-save").addEventListener("click", wizardSave);
  $("btn-copy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(asToml(formChanges()));
      $("copy-note").textContent = "copied";
    } catch {
      $("copy-note").textContent = "could not copy — select it and copy by hand";
    }
    setTimeout(() => ($("copy-note").textContent = ""), 3000);
  });
}

// A report of the last test session, as the on-air issue form takes it: copied to the
// clipboard as a link that opens the form pre-filled, so contributing a session is a
// paste and an attachment. The sidecar itself is attached by hand: a link cannot
// carry a file, and the operator decides whether the audio goes with it.
function contributeUrl(results, status, operator) {
  const path = results.path ?? {};
  const where = (call, grid) => (grid ? `${call} (${grid})` : call);
  const km = path.km == null ? "distance?" : `~${path.km} km`;
  const rig = [operator.rig, operator.power_w != null ? `${operator.power_w} W` : "", operator.antenna]
    .filter(Boolean)
    .join(", ");
  const ladder = (results.ladder ?? [])
    .map((r) => `mode ${r.mode}: ${r.decoded}/${r.frames} at ${r.snr_db ?? "?"} dB`)
    .join("; ");
  const transfer = (name, t) =>
    t ? `${name}: ${t.bytes} bytes in ${t.seconds} s (${t.bps} bit/s)` : `${name}: not run`;
  const outcome = [
    `Outcome: ${results.outcome ?? "still running"}`,
    results.probe
      ? `Probe: they hear us at ${results.probe.heard_there_db ?? "?"} dB, heard at ${results.probe.heard_here_db} dB`
      : "Probe: no answer",
    transfer("Message", results.message),
    transfer("File", results.file),
    ladder ? `Ladder: ${ladder}` : "Ladder: not run",
    "",
    (results.adjustments ?? []).length ? `Adjusted: ${results.adjustments.join("; ")}` : "",
    "",
    "Sidecar: attach the .json the test session wrote beside its .wav in the recordings folder (Help › Open the configuration folder). Attach the .wav too, zipped, if you are happy to share the audio.",
  ].join("\n");
  const params = new URLSearchParams({
    template: "on_air_report.yml",
    title: `Test session ${status.callsign ?? ""} ↔ ${results.remote}`,
    version: status.version ?? "",
    path: `${where(status.callsign ?? "", path.my_grid)} ↔ ${where(results.remote, path.their_grid)}, ${results.bandwidth_hz} Hz, ${results.started}, ${km}`,
    stations: rig,
    outcome,
  });
  return `https://github.com/KK4ODA/aether-hf/issues/new?${params}`;
}

async function contributeTestSession() {
  const line = $("contribute-note");
  const link = $("contribute-link");
  link.hidden = true;
  try {
    const [test, status] = await Promise.all([call("test.status"), call("status")]);
    if (!test.results) {
      line.textContent = "No test session has run yet: Session › Test session, with the other station's callsign in Call.";
      return;
    }
    const url = contributeUrl(test.results, status, liveConfig?.operator ?? {});
    link.href = url;
    link.hidden = false;
    await navigator.clipboard.writeText(url);
    line.textContent = "A link to the pre-filled report is on the clipboard and beside the button: open it, attach the .json, and send. Thank you.";
  } catch (error) {
    line.textContent = `Could not prepare the report: ${error.message ?? error}`;
  }
}

async function act(operation, description) {
  try {
    await operation();
    log(description);
    refreshStatus();
    return true;
  } catch (error) {
    log(error.message, true);
    return false;
  }
}

// ── panes ───────────────────────────────────────────────────────────

async function copyDiagnostics() {
  const note = $("diagnostics-note");
  note.textContent = "Collecting…";
  try {
    const bundle = await call("diagnostics");
    const text = JSON.stringify(bundle, null, 2);
    try {
      await navigator.clipboard.writeText(text);
      note.textContent = `Copied ${(text.length / 1024).toFixed(0)} kB to the clipboard.`;
    } catch {
      // no clipboard (a plain http page in some browsers): show it, so it can be selected
      $("log").textContent = text;
      note.textContent = "The clipboard is not available here; the bundle is shown below.";
    }
  } catch (error) {
    note.textContent = error.message;
  }
}

let recordingPath = null;

let recordingsDir = null;

function applyRecordingsDir(dir) {
  recordingsDir = dir;
  const line = $("recordings-dir");
  line.textContent = dir ?? "set a recordings folder, or none is written";
  const have = Boolean(dir);
  $("btn-open-recordings").disabled = !have;
  $("btn-copy-recordings").disabled = !have;
}

// The current session, or the last one this panel saw, in a word: who and how it ended.
let lastSessionText = null;
function applyLastSession(status) {
  const line = $("last-session");
  if (status.state === "connected" && status.remote) {
    lastSessionText = `${status.remote} — connected${status.role === "iss" ? ", sending" : status.role === "irs" ? ", receiving" : ""}`;
  } else if (status.state === "connecting" && status.remote) {
    lastSessionText = `${status.remote} — calling`;
  }
  if (lastSessionText) line.textContent = lastSessionText;
}

async function openRecordingsFolder() {
  if (!recordingsDir) return;
  // under the desktop shell the opener reveals it; a plain browser cannot, so the
  // path is copied instead and the note says what to do with it
  const opener = window.__TAURI__?.opener;
  if (opener?.openPath) {
    try {
      await opener.openPath(recordingsDir);
      return;
    } catch {
      // fall through to copying the path
    }
  }
  await copyRecordingsPath(true);
}

async function copyRecordingsPath(fallback = false) {
  if (!recordingsDir) return;
  try {
    await navigator.clipboard.writeText(recordingsDir);
    $("record-note").textContent = fallback
      ? "This build cannot open the folder for you — the path is on the clipboard; paste it into your file manager."
      : "Recordings folder path copied.";
  } catch {
    $("record-note").textContent = recordingsDir;
  }
}

async function copyReceived() {
  const text = $("incoming").innerText;
  try {
    await navigator.clipboard.writeText(text);
    $("send-note").textContent = "Received text copied.";
  } catch {
    $("send-note").textContent = "Could not copy — select the text and press Ctrl+C.";
  }
}

function applyRecording(recording) {
  recordingPath = recording?.path ?? null;
  const button = $("btn-record");
  button.textContent = recordingPath ? "Stop recording" : "Record";
  button.classList.toggle("danger", Boolean(recordingPath));
  if (recordingPath) {
    const name = recordingPath.split(/[\\/]/).pop();
    $("record-note").textContent = `● ${name} — ${Math.round(recording.seconds)} s`;
  } else if ($("record-note").textContent.startsWith("●")) {
    $("record-note").textContent = "";
  }
}

async function toggleRecording() {
  if (recordingPath) {
    try {
      const summary = await call("record.stop");
      $("record-note").textContent =
        `Saved ${summary.wav.split(/[\\/]/).pop()}: ${Math.round(summary.seconds)} s, ${summary.frames} frames, ${summary.decoded} decoded.`;
      log(`recording saved: ${summary.wav}`);
    } catch (error) {
      $("record-note").textContent = error.message;
    }
    applyRecording(null);
    refreshStatus();
    return;
  }
  const notes = $("record-notes").value.trim();
  try {
    const started = await call("record.start", notes ? { notes } : {});
    log(`recording ${started.path}`);
    applyRecording({ path: started.path, seconds: 0 });
  } catch (error) {
    $("record-note").textContent = error.message;
  }
}

// A lamp says its state in words as well as in colour, for a screen reader and for
// anyone who cannot tell the colours apart.
function setLamp(id, on, label) {
  const lamp = $(id);
  lamp.classList.toggle("on", on);
  if (lamp.getAttribute("aria-label") !== label) lamp.setAttribute("aria-label", label);
}

function log(message, bad = false) {
  const line = document.createElement("div");
  line.className = `log-line${bad ? " bad" : ""}`;
  const when = document.createElement("span");
  when.className = "when";
  when.textContent = new Date().toLocaleTimeString() + "  ";
  line.append(when, document.createTextNode(message));
  const pane = $("log");
  pane.append(line);
  while (pane.childElementCount > 500) pane.firstElementChild.remove();
  pane.scrollTop = pane.scrollHeight;
}

// The received pane shows reconstructed user text only. Payload bytes are decoded as
// UTF-8 with a strict, streaming decoder: a partial character at a frame boundary is
// held for the next frame, but a byte sequence that is not text — a binary or corrupted
// stream — throws rather than rendering as replacement-character soup, and is reported
// as a count instead. This is what keeps garbage out of the message window.
let rxDecoder = null;
let rxDropped = 0;

function resetReceived() {
  rxDecoder = new TextDecoder("utf-8", { fatal: true });
  rxDropped = 0;
}

function onReceivedData(base64) {
  const bytes = bytesFromBase64(base64);
  if (!bytes || bytes.length === 0) return;
  if (!rxDecoder) resetReceived();
  let text;
  try {
    text = rxDecoder.decode(bytes, { stream: true });
  } catch {
    // not text: a compressed or binary stream, or a corrupted one. Never render it.
    rxDropped += bytes.length;
    rxDecoder = new TextDecoder("utf-8", { fatal: true });
    noteDropped();
    return;
  }
  if (text) appendReceived(text);
}

function appendReceived(text) {
  const pane = $("incoming");
  // keep the reader's place and any selection if they have scrolled up to read
  const atBottom = pane.scrollTop + pane.clientHeight >= pane.scrollHeight - 4;
  pane.appendChild(document.createTextNode(text));
  if (atBottom) pane.scrollTop = pane.scrollHeight;
}

function noteDropped() {
  const pane = $("incoming");
  let tag = pane.querySelector(".rx-drop:last-child");
  if (!tag || tag !== pane.lastElementChild) {
    tag = document.createElement("span");
    tag.className = "rx-drop";
    pane.appendChild(tag);
  }
  tag.textContent = `\n[${rxDropped} bytes of non-text data were not shown]\n`;
  pane.scrollTop = pane.scrollHeight;
}

/// The form's answers as a configuration file, for somebody editing one by hand.
function asToml(changes) {
  const quote = (value) => `"${value}"`;
  const lines = [`callsign = ${quote(changes.callsign ?? "N0CALL")}`, "", "[audio]"];
  for (const [key, name] of [
    ["audio.input", "input"],
    ["audio.output", "output"],
  ]) {
    lines.push(
      changes[key]
        ? `${name} = ${quote(changes[key])}`
        : `# ${name} = "…"   # system default`,
    );
  }
  lines.push(`tx_level = ${changes["audio.tx_level"] ?? 0.25}`, "", "[ptt]");
  if (changes["ptt.kind"] === "rigctld") {
    lines.push(`kind = "rigctld"`, `address = ${quote(changes["ptt.address"] ?? "127.0.0.1:4532")}`);
  } else if (changes["ptt.kind"] === "cm108") {
    lines.push(
      `kind = "cm108"`,
      `device = ${quote(changes["ptt.device"])}`,
      `gpio = ${changes["ptt.gpio"] ?? 3}`,
    );
  } else if (changes["ptt.kind"] === "cat") {
    lines.push(
      `kind = "cat"`,
      `port = ${quote(changes["ptt.port"])}`,
      `protocol = ${quote(changes["ptt.protocol"])}`,
      `baud = ${changes["ptt.baud"]}`,
    );
    if (changes["ptt.civ_address"] != null) lines.push(`civ_address = ${changes["ptt.civ_address"]}`);
  } else if (changes["ptt.port"]) {
    lines.push(
      `kind = "serial"`,
      `port = ${quote(changes["ptt.port"])}`,
      `line = ${quote(changes["ptt.line"] ?? "rts")}`,
    );
  } else {
    lines.push(`kind = "none"   # VOX, or receive only`);
  }
  if (changes["operator.grid"] || changes["operator.rig"] || changes["operator.antenna"] || changes["operator.power_w"] != null) {
    lines.push("", "[operator]");
    if (changes["operator.grid"]) lines.push(`grid = ${quote(changes["operator.grid"])}`);
    if (changes["operator.rig"]) lines.push(`rig = ${quote(changes["operator.rig"])}`);
    if (changes["operator.power_w"] != null) lines.push(`power_w = ${changes["operator.power_w"]}`);
    if (changes["operator.antenna"]) lines.push(`antenna = ${quote(changes["operator.antenna"])}`);
  }
  lines.push("", "[radio]", "max_key_s = 30.0", "wait_for_clear = true");
  return lines.join("\n");
}

// ── base64, the way the control API uses it ─────────────────────────

function toBase64(text) {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

function bytesFromBase64(text) {
  try {
    const binary = atob(text);
    return Uint8Array.from(binary, (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
}

function clearNote(id) {
  const el = $(id);
  el.textContent = "";
  delete el.dataset.state;
}

// ── the splash ──────────────────────────────────────────────────────
//
// The logo, briefly, when the panel first opens — and not again on every reconnect after a
// restart, which is a plain refresh of the same instrument. A reader who has asked for less
// motion gets the mark for a moment and no fade.

function splash() {
  const overlay = $("splash");
  if (!overlay) return;
  let seen = false;
  try {
    seen = sessionStorage.getItem("aether-splashed") === "1";
    sessionStorage.setItem("aether-splashed", "1");
  } catch {
    // storage may be unavailable; the splash is then shown, which is the harmless case
  }
  if (seen) {
    overlay.remove();
    return;
  }
  const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  const dwell = reduced ? 700 : 1300;
  setTimeout(() => {
    overlay.classList.add("gone");
    setTimeout(() => overlay.remove(), reduced ? 0 : 300);
  }, dwell);
}

splash();
wire();
setLink(false);
connect();
