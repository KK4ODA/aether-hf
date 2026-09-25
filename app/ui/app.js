// Aether HF station panel.
//
// Talks to `aetherd` over the control API in `docs/spec/control-api.md`: a WebSocket at
// /v1 carrying requests correlated by id, and unsolicited events. Plain ES modules with no
// build step — see ADR-0005 — so the daemon can serve this directory as it stands and a
// gateway needs no Node toolchain to build.

import { createScopes, mountSignalBody, surface, tokens } from "./scopes.js";

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
    loadSessions();
    loadMemories();
    // devices first: the configuration selects among them, and a profile's guess must not
    // overwrite what the file says
    await loadDevices();
    await loadConfig();
    loadProfiles();
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
    else {
      // the refusal's code travels with it: a transmission the rules refused is logged as
      // the rules' ("regulatory"), with the decision the modem attached
      const error = new Error(frame.error?.message ?? "the modem refused");
      error.code = frame.error?.code ?? null;
      error.decision = frame.result?.decision ?? null;
      waiting.reject(error);
    }
  });

  socket.addEventListener("close", () => {
    setLink(false);
    scopes.stop();
    showSignalLive();
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
  renderProfiles();
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
      log(`${data.name}: ${data.detail}`, false, "state");
      if (data.name === "connected") {
        resetReceived();
        markSession(data.remote || data.detail);
        showBanner("connected", `CONNECTED — ${data.remote || data.detail}`);
        chime("up");
        focusComposer();
      } else if (data.name === "disconnected") {
        showBanner("ended", `SESSION ENDED — ${data.detail}`, 15000);
        chime("down");
      }
      refreshStatus();
      break;
    case "frame":
      onFrame(data);
      break;
    case "heard":
      noteHeard(data);
      break;
    case "sent":
      onSent(data);
      break;
    case "session":
      noteSession(data);
      break;
    case "profile":
      // the settings, the dials or the profiles changed: the star by the name follows
      applyProfiles(data);
      break;
    case "data":
      onReceivedData(data.data ?? "");
      break;
    case "log":
      log(data.detail ?? "", data.name === "error", data.name ?? "modem");
      // the probe's answer, or its absence, where the button is
      if (data.name === "probe") noteProbe(data.detail ?? "");
      if (data.name === "test") noteTest(data.detail ?? "");
      break;
    case "regulatory":
      onRegulatory(data);
      break;
    default:
      log(JSON.stringify(data), false, frame.event);
  }
}

// The probe's report is the modem's own sentence — "KK4XYZ hears us at 12 dB, heard at
// 14.0 dB" or "KK4XYZ: no answer" — shown as it is, beside the button that asked.
function noteTest(detail) {
  const line = $("test-result");
  line.textContent = `Test session: ${detail}`;
  line.dataset.state = detail.startsWith("aborted") || detail.startsWith("stopped") ? "warn" : "ok";
}

// ── the test session's progress ─────────────────────────────────────
// What the operator needs while a test runs: the step, out of the six; for a transfer, the
// bytes acknowledged of the total; on the ladder, the rung under test out of all of them,
// the fastest that has passed and the failures in a row that end it; the rung the link is
// using and how the other station hears this one; and the time — elapsed, and the most
// the budget leaves. No countdown: how long a step takes is the path's to say. The old
// line said "0 rungs" through three whole tests with ND1J that never reached the ladder,
// which read as a ladder that would not climb.

const TEST_STEPS = ["probe", "connect", "message", "ladder", "file", "disconnect"];
const TEST_STEP_NAMES = {
  probe: "probing",
  connect: "calling",
  message: "sending the message",
  ladder: "climbing the mode ladder",
  file: "sending the file",
  disconnect: "disconnecting",
};

function rungLabel(mode, name) {
  return name ? `rung ${mode} (${name})` : `rung ${mode}`;
}

function renderTestProgress(t) {
  const box = $("test-progress");
  if (!t) {
    box.hidden = true;
    return;
  }
  box.hidden = false;
  const index = TEST_STEPS.indexOf(t.step);
  const where = index >= 0 ? `step ${index + 1} of ${TEST_STEPS.length}` : t.step;
  const left = t.remaining_s != null ? ` · at most ${formatDuration(t.remaining_s)} left in the budget` : "";
  $("test-step").textContent =
    `Test with ${t.remote}: ${TEST_STEP_NAMES[t.step] ?? t.step} (${where}) · ${formatDuration(t.elapsed_s)} elapsed${left}`;

  const bar = $("test-bar");
  const label = $("test-bar-label");
  const ladder = t.ladder ?? null;
  if (t.transfer && t.transfer.bytes > 0) {
    bar.max = t.transfer.bytes;
    bar.value = t.transfer.acked;
    label.textContent = `${t.transfer.acked} of ${t.transfer.bytes} bytes acknowledged`;
  } else if (t.step === "ladder" && ladder && ladder.total > 0) {
    bar.max = ladder.total;
    bar.value = ladder.done;
    label.textContent = `${ladder.done} of ${ladder.total} rungs tried`;
  } else {
    bar.max = 1;
    bar.value = 0;
    label.textContent = "";
  }

  const rungs = $("test-ladder");
  if (ladder && (t.step === "ladder" || ladder.done > 0)) {
    const parts = [];
    if (ladder.testing != null) {
      parts.push(
        `Testing ${rungLabel(ladder.testing, ladder.testing_name)}, rung ${ladder.done + 1} of ${ladder.total}, ${ladder.frames} frame${ladder.frames === 1 ? "" : "s"}`,
      );
    }
    parts.push(
      ladder.highest_passed != null
        ? `highest passed: ${rungLabel(ladder.highest_passed, ladder.highest_passed_name)}`
        : "no rung passed yet",
    );
    parts.push(`failures in a row: ${ladder.failures_in_row} of ${ladder.failures_allowed}`);
    if (ladder.last) {
      const snr = ladder.last.snr_db != null ? ` at ${Math.round(ladder.last.snr_db)} dB` : "";
      const verdict = ladder.last.decoded * 2 >= ladder.last.frames ? "passed" : "failed";
      parts.push(
        `last: rung ${ladder.last.mode} ${verdict}, ${ladder.last.decoded}/${ladder.last.frames}${snr}`,
      );
    }
    rungs.textContent = parts.join(" · ");
    rungs.hidden = false;
  } else {
    rungs.hidden = true;
  }

  const link = t.link ?? null;
  if (link) {
    const heard =
      link.heard_there_db != null ? ` · ${t.remote} hears us at ${Math.round(link.heard_there_db)} dB` : "";
    $("test-link").textContent = `In use: ${rungLabel(link.rung, link.rung_name)}${heard}`;
  } else {
    $("test-link").textContent = "";
  }
}

function noteProbe(detail) {
  const line = $("probe-result");
  const unanswered = detail.endsWith("no answer");
  line.textContent = unanswered ? `${detail} — try again, or change band` : detail;
  line.dataset.state = unanswered ? "warn" : "ok";
}

// ── the session banner and chime ────────────────────────────────────
// A session coming up or ending is the one thing at the radio that must not be missed,
// and the state strip is small and on one tab: a banner across every tab says it in large
// type, and a chime says it out loud. A browser plays sound only once the page has been
// touched, so the first click or key unlocks it; until then the banner does the telling.

let chimeContext = null;
function unlockChime() {
  if (chimeContext) return;
  try {
    chimeContext = new (window.AudioContext || window.webkitAudioContext)();
  } catch {
    chimeContext = null;
  }
}
document.addEventListener("pointerdown", unlockChime, { once: true });
document.addEventListener("keydown", unlockChime, { once: true });

function chimeEnabled() {
  try {
    return localStorage.getItem("aether.chime") !== "off";
  } catch {
    return true;
  }
}

// two rising notes for a session up, two falling for one ended
function chime(kind) {
  if (!chimeEnabled()) return;
  playNotes(kind === "up" ? [[660, 0], [880, 0.16]] : [[660, 0], [440, 0.16]], 0.3, 0.24);
}

// `notes` are [hertz, start in seconds]; each rises to `peak` and dies away over `length`
function playNotes(notes, peak, length) {
  if (!chimeContext) return;
  if (chimeContext.state === "suspended") chimeContext.resume();
  const t0 = chimeContext.currentTime + 0.02;
  for (const [hz, at] of notes) {
    const osc = chimeContext.createOscillator();
    const gain = chimeContext.createGain();
    osc.type = "sine";
    osc.frequency.value = hz;
    gain.gain.setValueAtTime(0.0001, t0 + at);
    gain.gain.exponentialRampToValueAtTime(peak, t0 + at + 0.02);
    gain.gain.exponentialRampToValueAtTime(0.0001, t0 + at + length);
    osc.connect(gain).connect(chimeContext.destination);
    osc.start(t0 + at);
    osc.stop(t0 + at + length + 0.02);
  }
}

let bannerTimer = null;
// Show the banner in a state, pulsing once so the change is seen; `hideAfterMs` takes it
// down again — an ended session need not stay on the screen for ever.
function showBanner(state, text, hideAfterMs = 0) {
  const banner = $("session-banner");
  clearTimeout(bannerTimer);
  banner.dataset.state = state;
  $("session-banner-text").textContent = text;
  banner.hidden = false;
  banner.classList.remove("pulse");
  requestAnimationFrame(() => requestAnimationFrame(() => banner.classList.add("pulse")));
  if (hideAfterMs > 0) {
    bannerTimer = setTimeout(() => {
      banner.hidden = true;
    }, hideAfterMs);
  }
}

// Keep the banner true to the status the panel just read: a reload lands on a session
// already up, a call placed by a host program shows as calling, and an idle modem with
// nothing announced takes a stale banner down.
function syncBanner(status) {
  const banner = $("session-banner");
  const showing = banner.hidden ? "" : banner.dataset.state;
  if (status.state === "connected") {
    const role = { iss: ", sending", irs: ", receiving" }[status.role] ?? "";
    const text = `CONNECTED — ${status.remote || "?"}${role}`;
    if (showing === "connected") $("session-banner-text").textContent = text;
    else showBanner("connected", text);
  } else if (status.state === "connecting") {
    if (showing !== "calling") showBanner("calling", `CALLING ${status.remote || ""}…`);
  } else if (status.state === "disconnecting") {
    if (showing !== "calling") showBanner("calling", `CLOSING — ${status.remote || ""}`);
  } else if (showing === "connected" || showing === "calling") {
    banner.hidden = true;
  }
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
  syncBanner(status);

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
  applyKiss(status.kiss ?? null, host, status.datagrams ?? null);

  applyMetrics(status.metrics ?? {});
  renderCounters(status.counters ?? {});

  applyDial(status);
  applyRegulatory(status.regulatory ?? null);
  applyDeclaredDial(status);
  keyingSummary();
  applyAboutFromStatus(status);
  $("btn-connect").disabled = status.state !== "idle";
  $("btn-beacon").disabled = status.state !== "idle";
  $("btn-probe").disabled = status.state !== "idle";
  // one button: it starts a test session when the station is idle, and stops the one
  // that is running
  $("btn-test").textContent = status.test ? "Stop test" : "Test session";
  $("btn-test").disabled = !status.test && status.state !== "idle";
  testRunning = Boolean(status.test);
  renderTestProgress(status.test ?? null);
  if (status.remote) lastRemote = status.remote;
  reconcileSent(status);
  applyRecording(status.recording ?? null);
  applyFaults(status);
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
  applyTxPeak(metrics.tx_peak_dbfs);
  keyingSummary();
  applyPassband(metrics.rx_passband_hz, metrics.occupied_hz);
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
      // the largest level-over-floor the detector tested since the last sample: the decision
      // is made forty times a second, and the bar is one sample of it in twenty
      peak:
        metrics.transmitting === true || typeof metrics.excess_peak_db !== "number"
          ? null
          : floor + metrics.excess_peak_db,
      floor,
      tx: metrics.transmitting === true,
      rx: metrics.receiving === true,
      busy: metrics.channel_busy === true,
      // which path last lit the lamp — level, shape or frame — so the chart can say so
      // where the bars alone would contradict it
      why: typeof metrics.busy_reason === "string" ? metrics.busy_reason : null,
    });
    while (history.length && history[0].at < at - LEVEL_SPAN_MS) history.shift();
    drawChart();
    // the SNR chart's window slides with the clock even when no frame arrives
    drawStatusChart();
  }
  if (level !== null && level !== undefined && floor !== null && floor !== undefined) {
    $("d-busy").textContent = metrics.channel_busy ? "busy" : "clear";
    const delta = level - floor;
    const signed = (v) => `${v >= 0 ? "+" : ""}${v.toFixed(1)}`;
    $("d-busy-sub").textContent = `level ${level.toFixed(1)} · floor ${floor.toFixed(1)} dBFS · delta ${signed(delta)} of ${busyThresholdDb} dB`;
    // the peak the detector tested since the last reading, and which path last lit it —
    // the two things the reading itself cannot show
    const peak = typeof metrics.excess_peak_db === "number" ? `peak ${signed(metrics.excess_peak_db)} dB` : "";
    // the passband's shape: flat noise reads about 6 dB, a narrowband signal 15 and up,
    // and it is the one reading the receiver's AGC cannot compress
    const shape = typeof metrics.shape_db === "number" ? `shape ${metrics.shape_db.toFixed(1)} dB` : "";
    const why =
      metrics.busy_reason === "frame"
        ? "last lit by a decoded frame"
        : metrics.busy_reason === "shape"
          ? "last lit by the passband's shape"
          : metrics.busy_reason === "level"
            ? "last lit by the level"
            : "";
    $("d-busy-why").textContent = [peak, shape, why].filter(Boolean).join(" · ");
  }
  // the lamp says which path lit it: the level threshold, or a frame acquired
  const why =
    metrics.busy_reason === "frame"
      ? "Channel busy: a frame decoded"
      : metrics.busy_reason === "shape"
        ? "Channel busy: a narrowband signal is in the passband"
        : metrics.busy_reason === "level"
          ? `Channel busy: the level crossed the threshold (${busyThresholdDb} dB over the floor)`
          : "Channel busy";
  setLamp("lamp-busy", metrics.channel_busy === true, metrics.channel_busy ? why : "Channel clear");
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
    if (signalDocked) scopes.frame();
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
function chartFrame(id, fallbackWidth, height, low, high, legend = null) {
  const s = surface(id, fallbackWidth, height);
  const c = tokens();
  s.ctx.clearRect(0, 0, s.width, s.height);
  s.ctx.font = `10.5px ${c.numerals}`;
  s.ctx.textBaseline = "middle";
  const plotWidth = s.width - CHART_LEFT - CHART_RIGHT;
  // a legend wider than the chart wraps, and the plot starts under its last row rather
  // than behind it — the rows are counted here so the y scale knows where it may begin
  const rows = legend ? legendRows(s.ctx, plotWidth, legend) : 1;
  const top = CHART_TOP + (rows - 1) * LEGEND_ROW;
  const bottom = height - CHART_BOTTOM;
  const y = (value) => top + (1 - (value - low) / (high - low)) * (bottom - top);
  return { ...s, c, plotWidth, bottom, top, y, legend };
}

/// How many rows a legend takes at this plot width.
function legendRows(ctx, plotWidth, items) {
  const right = CHART_LEFT + plotWidth;
  let at = CHART_LEFT;
  let rows = 1;
  for (const [, text] of items) {
    const width = ctx.measureText(text).width;
    if (at > CHART_LEFT && at + width > right) {
      at = CHART_LEFT;
      rows += 1;
    }
    at += width + 14;
  }
  return rows;
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
    ctx.moveTo(x(now - back), f.top ?? CHART_TOP);
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
  // a legend wider than the chart wraps to a second row rather than running off the
  // right edge; the frame was told the items, so the plot already starts below it
  const right = CHART_LEFT + f.plotWidth;
  let at = CHART_LEFT;
  let row = 8;
  for (const [colour, text] of items) {
    const width = ctx.measureText(text).width;
    if (at > CHART_LEFT && at + width > right) {
      at = CHART_LEFT;
      row += LEGEND_ROW;
    }
    ctx.fillStyle = colour;
    ctx.fillText(text, at, row);
    at += width + 14;
  }
}

/// Height of one legend row, in canvas pixels.
const LEGEND_ROW = 11;

// ── the Status chart area: SNR by frame, or connection speed ────────
//
// One frame holds two views the operator switches between. Only the visible canvas is
// drawn — a hidden one has no width to size against — so every redraw goes through
// drawStatusChart.

let statusChart = "speed";

// Speed is the view the Status tab opens on. A choice is remembered only when the operator
// makes one: the key before beta.60 (`aether.statuschart`) was written on every load, so its
// "snr" said what the default had been rather than what anybody chose — it is dropped, not
// carried over, and the new key is written only by a click.
const STATUS_CHART_KEY = "aether.chart";

function storedStatusChart() {
  try {
    localStorage.removeItem("aether.statuschart");
    return localStorage.getItem(STATUS_CHART_KEY) === "snr" ? "snr" : "speed";
  } catch {
    return "speed";
  }
}

function drawStatusChart() {
  if (statusChart === "speed") drawSpeedChart();
  else drawSnrChart();
}

function selectStatusChart(which, chosen = false) {
  statusChart = which === "snr" ? "snr" : "speed";
  const speed = statusChart === "speed";
  $("chart-snr").hidden = speed;
  $("chart-speed").hidden = !speed;
  $("chart-tab-snr").setAttribute("aria-selected", String(!speed));
  $("chart-tab-speed").setAttribute("aria-selected", String(speed));
  if (chosen) {
    try {
      localStorage.setItem(STATUS_CHART_KEY, statusChart);
    } catch {
      // a browser that keeps nothing keeps nothing
    }
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
    // The label goes where the readings are not. A session that sits right at its mode's
    // threshold — which is what a rate controller aims for — puts every point on this
    // line, so a fixed spot beside it is a collision. Four places are tried, either end
    // of the line, above and below, and the one with the fewest points under it wins;
    // a background keeps the words legible even when nowhere is empty.
    const text = `mode ${currentMode} needs ${threshold.toFixed(1)} dB`;
    const width = ctx.measureText(text).width;
    const yLine = f.y(threshold);
    const points = [...snrHistory, ...peerHistory].map((point) => [x(point.at), f.y(point.snr)]);
    const candidates = [
      { left: CHART_LEFT + 4, y: yLine - 7 },
      { left: CHART_LEFT + 4, y: yLine + 8 },
      { left: CHART_LEFT + f.plotWidth - 4 - width, y: yLine - 7 },
      { left: CHART_LEFT + f.plotWidth - 4 - width, y: yLine + 8 },
    ];
    const conflicts = ({ left, y }) =>
      points.filter(([px, py]) => px >= left - 3 && px <= left + width + 3 && Math.abs(py - y) <= 7).length;
    const spot = candidates.reduce((best, next) => (conflicts(next) < conflicts(best) ? next : best));
    ctx.fillStyle = withAlpha(c.plot, 0.85);
    ctx.fillRect(spot.left - 3, spot.y - 6, width + 6, 12);
    ctx.textAlign = "left";
    ctx.fillStyle = c.ink2;
    ctx.fillText(text, spot.left, spot.y);
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
  datagram: "KISS frame",
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
  const hero = $("dial-hero");
  const heroSub = $("dial-hero-sub");
  if (status.frequency_hz) {
    reading.textContent = `radio: ${formatHz(status.frequency_hz)} Hz`;
    hero.textContent = `${formatHz(status.frequency_hz)} Hz`;
    heroSub.textContent = "";
    if (status.frequency_hz !== lastDialHz) {
      lastDialHz = status.frequency_hz;
      const match = memories.find((m) => Math.abs(m.hz - status.frequency_hz) <= 10);
      if (match) $("memory").value = String(match.hz);
    }
  } else if (!canTune && liveConfig?.regulatory?.dial_hz) {
    const hz = liveConfig.regulatory.dial_hz;
    reading.textContent = `declared: ${formatHz(hz)} Hz`;
    hero.textContent = `${formatHz(hz)} Hz`;
    heroSub.textContent = "declared — this radio cannot report its dial";
  } else if (canTune) {
    reading.textContent = "radio: no reading yet";
    hero.textContent = "—";
    heroSub.textContent = "no reading from the radio yet";
  } else {
    reading.textContent = "tuning needs CAT or rigctld keying (Setup step 2)";
    hero.textContent = "—";
    heroSub.textContent = "reading the dial needs CAT or rigctld keying (Setup step 2)";
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
    $("dial-hero").textContent = `${formatHz(hz)} Hz`;
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
    const actions = cell(button);
    if (station.connected || sessionList.some((s) => s.remote === station.callsign)) {
      const history = document.createElement("button");
      history.className = "small ghost";
      history.textContent = "Sessions";
      history.title = `Show only the sessions with ${station.callsign}, below`;
      history.addEventListener("click", () => showSessionsWith(station.callsign));
      actions.append(" ", history);
    }
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

// ── the sessions ────────────────────────────────────────────────────
// One line per session (`sessions.list`), newest first, beside the stations heard, which
// keep one line per callsign — the modem's history, so it is the same from every panel. A
// `session` event brings the one that just ended.

let sessionList = [];
let sessionsFilter = null;

async function loadSessions() {
  try {
    const result = await call("sessions.list");
    sessionList = result.sessions ?? [];
  } catch {
    return;
  }
  renderSessions();
  renderHeard();
}

function noteSession(entry) {
  if (!entry || !entry.remote) return;
  sessionList.unshift(entry);
  renderSessions();
  renderHeard();
}

function showSessionsWith(callsign) {
  sessionsFilter = callsign;
  renderSessions();
  $("sessions-head").scrollIntoView({ behavior: "smooth", block: "start" });
}

function renderSessions() {
  const rows = sessionsFilter ? sessionList.filter((s) => s.remote === sessionsFilter) : sessionList;
  const note = $("sessions-note");
  if (sessionsFilter) {
    note.textContent = `with ${sessionsFilter}: ${rows.length} session${rows.length === 1 ? "" : "s"}`;
  } else {
    note.textContent = sessionList.length ? `${sessionList.length} in all` : "";
  }
  $("btn-sessions-all").hidden = !sessionsFilter;
  $("btn-sessions-clear").disabled = sessionList.length === 0;
  $("sessions-empty").hidden = rows.length > 0;
  $("sessions-empty").textContent = sessionsFilter
    ? `No session with ${sessionsFilter} in the history.`
    : "No session yet. Each one is added here when it ends.";
  $("sessions-table").parentElement.hidden = rows.length === 0;
  const body = $("sessions");
  body.replaceChildren();
  const db = (value) => (value == null ? "—" : `${value.toFixed(1)}`);
  for (const s of rows) {
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
    cell(stamp(s.started_ms), "numerals", relative(s.started_ms));
    cell(s.remote, "call", `The other station: ${s.remote}`);
    cell(formatDuration(s.duration_s), "num", `${Math.round(s.duration_s)} s`);
    cell(
      s.role === "caller" ? "out" : "in",
      "",
      s.role === "caller" ? `This station called ${s.remote}` : `${s.remote} called this station`,
    );
    cell(
      `${s.frequency_hz ? formatHz(s.frequency_hz) : "—"} · ${s.bandwidth_hz} Hz`,
      "num",
      s.frequency_hz ? `Dial ${formatHz(s.frequency_hz)} Hz, ${s.bandwidth_hz} Hz wide` : `The radio gave no dial; ${s.bandwidth_hz} Hz wide`,
    );
    cell(
      `${formatBytes(s.bytes_acked)} / ${formatBytes(s.bytes_received)}`,
      "num",
      `Handed to the link: ${s.bytes_sent} bytes; acknowledged by ${s.remote}: ${s.bytes_acked} bytes (after compression); received: ${s.bytes_received} bytes`,
    );
    const rung = (value) => (value == null ? "—" : String(value));
    const named = (value) => {
      if (value == null) return "none";
      const mode = modeTable[value];
      return mode && mode.name ? `rung ${value} (${mode.name} on the ${s.bandwidth_hz} Hz air)` : `rung ${value}`;
    };
    // A direction with no traffic has nothing to show, and says so: in a session the other
    // station called and sent, this one only acknowledged — no rung out — and a station says
    // how it hears the other only when it acknowledges what that one sent.
    const pair = (out, back) => {
      const span = document.createDocumentFragment();
      const first = document.createElement("span");
      first.textContent = out.text;
      if (out.muted) first.className = "muted";
      span.append(first, ` / ${back}`);
      return span;
    };
    const sentNothing = !s.bytes_sent;
    cell(
      pair(
        s.top_rung_sent == null ? { text: sentNothing ? "none sent" : "—", muted: true } : { text: rung(s.top_rung_sent) },
        rung(s.top_rung_heard),
      ),
      "num",
      s.top_rung_sent == null && sentNothing
        ? `This station sent no data in this session — it only acknowledged what ${s.remote} sent — so there is no rung out; fastest decoded from ${s.remote}: ${named(s.top_rung_heard)}`
        : `Fastest rung sent: ${named(s.top_rung_sent)}; fastest decoded from ${s.remote}: ${named(s.top_rung_heard)}`,
    );
    cell(
      pair(
        s.heard_there_db == null ? { text: "not said", muted: true } : { text: db(s.heard_there_db) },
        `${db(s.snr_db)} dB`,
      ),
      "num",
      s.heard_there_db == null
        ? `${s.remote} did not say how it heard this station: a station says that in its acknowledgements of what the other sends${sentNothing ? ", and this station sent nothing" : ""}; its last frame here was ${db(s.snr_db)} dB, the best ${db(s.best_snr_db)} dB`
        : `${s.remote} last said it heard this station at ${db(s.heard_there_db)} dB; its last frame here was ${db(s.snr_db)} dB, the best ${db(s.best_snr_db)} dB`,
    );
    const ended = cell(s.end, "", s.recording ? `Recorded as ${s.recording}` : "Not recorded");
    if (s.test) {
      const pill = document.createElement("span");
      pill.className = "act";
      pill.dataset.activity = "test";
      pill.textContent = "test";
      pill.title = "A Test session: its report is in the recording's sidecar";
      ended.append(" ", pill);
    }
    body.append(row);
  }
}

async function clearSessions() {
  if (!window.confirm("Forget every session in the history? The recordings and the stations heard are not touched.")) return;
  try {
    const result = await call("sessions.clear");
    sessionList = [];
    sessionsFilter = null;
    renderSessions();
    renderHeard();
    $("sessions-note").textContent = `Forgot ${result.cleared ?? 0}.`;
  } catch (error) {
    $("sessions-note").textContent = error.message;
  }
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

// ── diagnostics: signal analysis, docked or in its own window ───────
//
// The constellation, spectrum and waterfall are one card (scopes.js), drawn here on the
// Diagnostics tab or, undocked, in a window of their own (signal.html) that stays in view
// whatever tab this panel shows. Only one of the two draws at a time. Under the desktop shell
// the shell owns that window: it opens it, brings it forward rather than opening a second,
// closes it when asked and says when it has gone. In a browser it is a named pop-up — a
// second Undock finds the same window — and the page in it says on a BroadcastChannel that
// it is there, so a panel reloaded meanwhile still knows the card is out.

const scopes = createScopes({
  call,
  connected: () => socket !== null && socket.readyState === WebSocket.OPEN,
  modeName: (index) => modeTable[index]?.name ?? null,
  canPersist: () => Boolean(liveConfig?.panel),
  onActivity: () => showSignalLive(),
});

let signalDocked = true;
let signalPopup = null;
let signalPoll = null;
let signalSeenAt = 0;
let signalAnswerTimer = null;
let signalChannel = null;

const shellEvents = () => window.__TAURI__?.event ?? null;

function scopesWanted() {
  const up = socket !== null && socket.readyState === WebSocket.OPEN;
  if (up && signalDocked && panelShown("diagnostics")) scopes.start();
  else scopes.stop();
}

function showSignalLive() {
  const mark = $("signal-live");
  const live = scopes.running();
  mark.dataset.live = String(live || !signalDocked);
  mark.textContent = !signalDocked ? "in its own window" : live ? "live" : "paused";
}

function signalNote(text) {
  $("signal-note").textContent = text;
  $("signal-note").hidden = !text;
}

function setSignalDocked(docked) {
  signalDocked = docked;
  $("signal-body").hidden = !docked;
  $("signal-away").hidden = docked;
  $("btn-undock").hidden = !docked;
  signalNote("");
  // the window may have changed the waterfall's settings while it had the card
  if (docked) scopes.reload();
  scopesWanted();
  showSignalLive();
}

async function undockSignal() {
  signalNote("");
  const events = shellEvents();
  if (events?.emit) {
    try {
      await events.emit("panel-signal", { action: "undock" });
      // the shell answers with `signal-window`; a shell older than the panel says nothing
      clearTimeout(signalAnswerTimer);
      signalAnswerTimer = setTimeout(() => {
        if (signalDocked) {
          signalNote("This version of the desktop application cannot undock the card: update it, or open the panel in a browser.");
        }
      }, 2500);
      return;
    } catch {
      // not allowed here: a browser window will do
    }
  }
  const popup = window.open("signal.html", "aether-signal", "popup,width=860,height=640");
  if (!popup) {
    signalNote("The browser blocked the window: allow pop-ups for this page, then press Undock again.");
    return;
  }
  signalPopup = popup;
  signalSeenAt = Date.now();
  setSignalDocked(false);
  watchSignalWindow();
}

// A browser says nothing when a window closes: look twice a second — at the window itself
// when this page opened it, at its announcements when an earlier page did.
function watchSignalWindow() {
  clearInterval(signalPoll);
  signalPoll = setInterval(() => {
    const gone = signalPopup ? signalPopup.closed : Date.now() - signalSeenAt > 5000;
    if (!gone) return;
    clearInterval(signalPoll);
    signalPoll = null;
    signalPopup = null;
    setSignalDocked(true);
  }, 500);
}

async function dockSignal() {
  const events = shellEvents();
  if (events?.emit && signalPopup === null) {
    try {
      await events.emit("panel-signal", { action: "dock" });
      return; // the shell closes the window and says so
    } catch {
      // fall through: close it from here
    }
  }
  if (signalPopup && !signalPopup.closed) signalPopup.close();
  else signalChannel?.postMessage({ dock: true });
  signalPopup = null;
  clearInterval(signalPoll);
  signalPoll = null;
  setSignalDocked(true);
}

function wireSignalCard() {
  mountSignalBody($("signal-body"));
  scopes.wire();
  $("btn-undock").addEventListener("click", undockSignal);
  $("btn-signal-back").addEventListener("click", dockSignal);
  // the undocked window changed the waterfall: follow it
  window.addEventListener("storage", (event) => {
    if (event.key === "aether.waterfall") scopes.reload();
  });
  const events = shellEvents();
  if (events?.listen) {
    events
      .listen("signal-window", (event) => {
        clearTimeout(signalAnswerTimer);
        setSignalDocked(event.payload?.open !== true);
      })
      .then(() => events.emit("panel-signal", { action: "state" }))
      .catch(() => {});
    return;
  }
  try {
    signalChannel = new BroadcastChannel("aether-signal");
  } catch {
    return;
  }
  signalChannel.addEventListener("message", (event) => {
    if (event.data?.alive === true) {
      signalSeenAt = Date.now();
      if (signalDocked) {
        setSignalDocked(false);
        watchSignalWindow();
      }
    } else if (event.data?.alive === false && signalPopup === null) {
      clearInterval(signalPoll);
      signalPoll = null;
      setSignalDocked(true);
    }
  });
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
//
// The busy lamp has three paths and the level is only one of them: behind a receiver's
// AGC a narrowband signal sits within a decibel or two of the floor and is caught by the
// passband's shape, and a decoded frame marks the channel on its own. The bars and the
// threshold line show the level path alone, so a band along the top of the plot says
// when the lamp was lit and in which path's colour, and a peak is drawn red only when it
// is over the line — otherwise the chart shows bars under the line with the lamp on and
// looks wrong when it is right.

const history = [];
const BUSY_BAND = 4;

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
    high = Math.max(high, point.floor + busyThresholdDb + 6, point.level ?? point.floor, point.peak ?? point.floor);
  }
  low = Math.floor((low - 3) / 5) * 5;
  high = Math.ceil((high + 3) / 5) * 5;
  const legend = [
    [null, "level, dBFS"],
    [null, "▍ receiving"],
    [null, "▍ transmitting"],
    [null, "- - noise floor"],
    [null, "· · busy threshold"],
    [null, "▏ peak that tripped it"],
    [null, "▬ busy: level"],
    [null, "▬ busy: shape"],
    [null, "▬ busy: frame"],
  ];
  const f = chartFrame("chart", 450, 180, low, high, legend);
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
      // the peak the detector tested in this half second, above the sampled bar. Red only
      // when it is over the line and the channel was busy — a peak that tripped it, or
      // helped to; a peak under the line is grey whatever the lamp says, and the band at
      // the top names the path that lit it when the level did not
      if (point.peak !== null && point.peak !== undefined && point.peak > point.level) {
        const crossed = point.peak - point.floor >= busyThresholdDb - 1e-6;
        ctx.fillStyle = point.busy && crossed ? c.error : withAlpha(c.ink, 0.5);
        const peakY = Math.min(f.y(point.peak), top - 1);
        ctx.fillRect(x0, peakY, bar, Math.max(1, top - peakY));
      }
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
      ctx.moveTo(at, f.top);
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

  // the busy threshold: the floor plus the configured margin, the line the lamp answers to
  ctx.strokeStyle = withAlpha(c.error, 0.8);
  ctx.lineWidth = 1;
  ctx.setLineDash([2, 3]);
  ctx.beginPath();
  history.forEach((point, index) => {
    const y = f.y(point.floor + busyThresholdDb);
    if (index === 0) ctx.moveTo(x(point.at), y);
    else ctx.lineTo(x(point.at), y);
  });
  ctx.stroke();
  ctx.setLineDash([]);

  // the band along the top while the channel was busy, in the colour of the path that
  // marked it: red for the level (the line crossed), the lamp's own orange for the
  // passband's shape, and the decoded-frame teal of the SNR chart for a frame. Slot-wide,
  // so consecutive samples join into one span
  const reasonColour = { level: c.error, shape: c.busy, frame: c.accent };
  for (const point of history) {
    if (!point.busy) continue;
    ctx.fillStyle = reasonColour[point.why] ?? c.busy;
    ctx.fillRect(x(point.at) - slot, f.top, slot, BUSY_BAND);
  }

  const colours = [c.ink2, c.rx, c.tx, c.ink2, c.error, c.error, c.error, c.busy, c.accent];
  drawLegend(f, legend.map(([, text], i) => [colours[i], text]));
}

window.addEventListener("resize", () => {
  drawChart();
  drawStatusChart();
});
document.addEventListener("visibilitychange", scopesWanted);

// ── capabilities and devices ────────────────────────────────────────

/// The fastest-mode list, from the mode table the modem reports. The table is the
/// running waveform's ladder; when the operator picks the other bandwidth the list shrinks
/// or grows to that ladder's size (fifteen rungs at 500 Hz, twenty at 2300) and the modem
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
  fillKissRungs();
  const usable = new Set(caps.usable_modes ?? []);
  const body = $("modes");
  body.replaceChildren();
  for (const mode of modeTable) {
    const row = document.createElement("tr");
    row.dataset.usable = String(usable.has(mode.index));
    for (const [text, numeric] of [
      [String(mode.index), true],
      // a floor rung is the tone floor's (ADR-0013): its bytes are per 5.4 s frame
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
  select($("radio-max-mode"), String(radio.max_mode ?? 19));
  $("radio-compress").checked = radio.compress === true;
  $("radio-wait").checked = radio.wait_for_clear !== false;
  $("radio-busy-db").value = String(radio.busy_threshold_db ?? 6);
  busyThresholdDb = Number(radio.busy_threshold_db ?? 6);
  $("radio-max-key").value = String(radio.max_key_s ?? 30);
  $("radio-cwid").checked = radio.cw_id === true;
  $("radio-cwid-interval").value = String(radio.cw_id_interval_s ?? 600);
  $("radio-cwid-wpm").value = String(radio.cw_id_wpm ?? 20);
  // the rules this station operates under (ADR-0018): nothing is assumed, and a field the
  // file leaves empty stays on "choose…"
  const rules = liveConfig.regulatory ?? {};
  select($("reg-profile"), rules.profile ?? "");
  select($("reg-control"), rules.control ?? "");
  select($("reg-license"), rules.license_class ?? "");
  select($("reg-sideband"), rules.sideband ?? "usb");
  $("reg-margin").value = String(rules.edge_margin_hz ?? 50);
  $("reg-band-plan").checked = rules.band_plan !== false;
  $("reg-log-permitted").checked = rules.log_permitted === true;
  // the operator's saved Interface choice wins over a guess; without one, the file's
  // capture device says which interface this station is, better than whatever else is
  // plugged in. Either way a choice already made by hand this session is left alone.
  const savedProfile = liveConfig.panel?.interface;
  const savedIndex = savedProfile
    ? PROFILES.findIndex((p) => p.id === savedProfile)
    : -1;
  if ($("wz-profile").options.length > 0) showInterface(savedIndex);
  select($("update-channel"), liveConfig.update?.channel ?? "stable");
  $("update-check").checked = liveConfig.update?.check !== false;
  $("record-auto").checked = liveConfig.record?.auto === true;
  $("record-standing").value = liveConfig.record?.notes ?? "";
  $("host-enabled").checked = liveConfig.host?.enabled === true;
  $("host-port").value = String(portOf(liveConfig.host?.bind) ?? 8300);
  const kiss = liveConfig.kiss ?? {};
  $("kiss-enabled").checked = kiss.enabled === true;
  $("kiss-host").value = hostOf(kiss.bind) || "127.0.0.1";
  $("kiss-port").value = String(portOf(kiss.bind) ?? 8100);
  kissRungWanted = String(kiss.rung ?? 1);
  select($("kiss-rung"), kissRungWanted);
  $("kiss-wait").checked = kiss.wait_for_clear !== false;
  scopes.take(liveConfig.panel?.waterfall);
  noteMissingDevices();
  // the file's devices, not the profile's guess, are what the warning should be about
  checkRates();
  checkRulesStep();
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
  // the KISS port (live keys: it opens, moves or closes when saved)
  changes["kiss.enabled"] = $("kiss-enabled").checked;
  const kissPort = Number($("kiss-port").value);
  if (Number.isInteger(kissPort) && kissPort > 0 && kissPort <= 65535) {
    changes["kiss.bind"] = joinAddress($("kiss-host").value.trim() || "127.0.0.1", kissPort);
  }
  changes["kiss.rung"] = Number($("kiss-rung").value);
  changes["kiss.wait_for_clear"] = $("kiss-wait").checked;
  // the rules (live keys): an empty choice is sent as empty, and blocks transmitting
  changes["regulatory.profile"] = $("reg-profile").value;
  changes["regulatory.control"] = $("reg-control").value;
  changes["regulatory.license_class"] = $("reg-license").value;
  changes["regulatory.sideband"] = $("reg-sideband").value;
  const margin = numberIn("reg-margin");
  if (margin !== null && $("reg-margin").value.trim() !== "") changes["regulatory.edge_margin_hz"] = margin;
  changes["regulatory.band_plan"] = $("reg-band-plan").checked;
  changes["regulatory.log_permitted"] = $("reg-log-permitted").checked;
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

// ── profiles ────────────────────────────────────────────────────────
//
// A profile is every portable setting under a name, kept by the daemon beside its
// configuration (`profile.*` in docs/spec/control-api.md). The panel is a client of
// those methods and nothing more: it never reads or writes a profile file itself, so
// what a profile holds is decided in one place. The `PROFILES` further down are the
// radio-interface presets of Setup step 2, an older use of the word.

let profiles = { active: null, name: null, dirty: null, list: [], dir: "" };
// what the name form is for once OK is pressed: "save-as", "new", "rename",
// "duplicate" or "delete"
let profilePurpose = null;
// what to do once the unsaved-changes question is answered
let profileAfterPrompt = null;
// the devices the last profile load could not find, with the daemon's suggestions
let profileMissing = [];

function noteProfile(text, state) {
  const note = $("profile-note");
  note.textContent = text;
  if (state) note.dataset.state = state;
  else delete note.dataset.state;
}

async function loadProfiles() {
  try {
    applyProfiles(await call("profile.list"));
  } catch (error) {
    // a daemon without a configuration file keeps no profiles; the bar says so and
    // offers nothing
    profiles = { active: null, name: null, dirty: null, list: [], dir: "" };
    renderProfiles();
    noteProfile(error.message, "warn");
  }
}

function applyProfiles(status) {
  profiles = {
    active: status.active ?? null,
    name: status.name ?? null,
    dirty: status.dirty ?? null,
    list: status.profiles ?? [],
    dir: status.dir ?? "",
  };
  renderProfiles();
}

function renderProfiles() {
  const select = $("profile-select");
  select.replaceChildren();
  if (!profiles.active) {
    const none = document.createElement("option");
    none.value = "";
    none.textContent = profiles.list.length ? "— pick a profile —" : "— no profile saved yet —";
    select.append(none);
  }
  for (const entry of profiles.list) {
    const option = document.createElement("option");
    option.value = entry.id;
    const active = entry.id === profiles.active;
    option.textContent = `${entry.name}${active && profiles.dirty ? " *" : ""}`;
    if (entry.error) {
      option.textContent += " — cannot be read";
      option.disabled = true;
    }
    option.title = entry.error
      ? `${entry.path}: ${entry.error}`
      : `${entry.path}${entry.modified ? ` — saved ${entry.modified}` : ""}`;
    select.append(option);
  }
  select.value = profiles.active ?? "";
  select.dataset.dirty = String(profiles.dirty === true);
  $("profile-label").dataset.dirty = String(profiles.dirty === true);
  select.title = profiles.dirty
    ? `${profiles.name}: changed since it was saved — Save writes the changes to it`
    : "The saved profile these settings come from; pick another to switch the station to it";
  const connected = socket && socket.readyState === WebSocket.OPEN;
  const hasActive = Boolean(profiles.active);
  $("profile-save").disabled = !connected || !hasActive;
  $("profile-save").title = hasActive
    ? profiles.dirty
      ? `Write the changes to ${profiles.name}`
      : `${profiles.name} is up to date`
    : "No profile is active: use Save as…";
  $("profile-rename").disabled = !hasActive;
  $("profile-duplicate").disabled = !hasActive;
  $("profile-export").disabled = !connected;
  $("profile-delete").disabled = profiles.list.filter((p) => p.id !== profiles.active).length === 0;
  for (const id of ["profile-save-as", "profile-import", "profile-new"]) $(id).disabled = !connected;
}

/// The form for a name (or, for a delete, a choice), with what OK will do.
function openProfileForm(purpose) {
  profilePurpose = purpose;
  $("profile-more").open = false;
  const form = $("profile-name-form");
  const name = $("profile-name");
  const pick = $("profile-pick");
  const labels = {
    "save-as": ["Save as", "A name for the new profile", "OK"],
    new: ["New profile", "A name for the new profile — it starts from the defaults", "Create"],
    rename: ["Rename to", "The profile's new name", "Rename"],
    duplicate: ["Copy as", "A name for the copy", "Copy"],
    delete: ["Delete", "Which profile to delete", "Delete"],
  };
  const [label, hint, ok] = labels[purpose];
  $("profile-name-label").textContent = label;
  $("profile-name-ok").textContent = ok;
  $("profile-name-ok").classList.toggle("danger", purpose === "delete");
  $("profile-name-ok").classList.toggle("primary", purpose !== "delete");
  name.hidden = purpose === "delete";
  pick.hidden = purpose !== "delete";
  name.title = hint;
  if (purpose === "delete") {
    pick.replaceChildren();
    for (const entry of profiles.list.filter((p) => p.id !== profiles.active)) {
      const option = document.createElement("option");
      option.value = entry.id;
      option.textContent = entry.name;
      pick.append(option);
    }
  } else {
    name.value = purpose === "rename" ? (profiles.name ?? "") : "";
    if (purpose === "duplicate" && profiles.name) name.value = `${profiles.name} copy`;
  }
  form.hidden = false;
  (purpose === "delete" ? pick : name).focus();
  if (purpose !== "delete") name.select();
}

function closeProfileForm() {
  $("profile-name-form").hidden = true;
  profilePurpose = null;
}

async function profileFormOk() {
  const purpose = profilePurpose;
  const name = $("profile-name").value.trim();
  if (purpose !== "delete" && !name) {
    $("profile-name").focus();
    return;
  }
  closeProfileForm();
  try {
    if (purpose === "save-as") {
      applyProfiles(await call("profile.save", { name }));
      noteProfile(`Saved as ${name}, now the active profile.`);
      log(`profile saved as ${name}`);
    } else if (purpose === "new") {
      const answer = await call("profile.create", { name });
      await afterProfileLoad(answer, `New profile ${name}: the defaults, with your callsign and operator details.`);
    } else if (purpose === "rename") {
      applyProfiles(await call("profile.rename", { id: profiles.active, name }));
      noteProfile(`Renamed to ${name}.`);
    } else if (purpose === "duplicate") {
      applyProfiles(await call("profile.duplicate", { id: profiles.active, name }));
      noteProfile(`Copied to ${name}. The station stays on ${profiles.name}; pick ${name} to switch.`);
    } else if (purpose === "delete") {
      const id = $("profile-pick").value;
      if (!id) return;
      const entry = profiles.list.find((p) => p.id === id);
      if (!window.confirm(`Delete the profile ${entry?.name ?? id}? Its file is removed; the running settings are not touched.`)) return;
      applyProfiles(await call("profile.delete", { id }));
      noteProfile(`Deleted ${entry?.name ?? id}.`);
    }
  } catch (error) {
    noteProfile(error.message, "error");
    log(error.message, true);
  }
}

/// Everything a load reported, in the operator's terms: what was left out, what could
/// not be found, what needs a restart — and the restart itself, when there is somebody
/// to do it.
async function afterProfileLoad(answer, lead) {
  applyProfiles(answer);
  const report = answer.report ?? {};
  profileMissing = report.missing_hardware ?? [];
  const parts = [lead];
  if ((report.unknown ?? []).length) {
    parts.push(
      `Left out — this version has no setting called ${report.unknown.join(", ")}${answer.aether_version ? ` (the profile was written by Aether HF ${answer.aether_version})` : ""}.`,
    );
  }
  if ((report.invalid ?? []).length) {
    parts.push(`Kept at the default: ${report.invalid.map((r) => r.reason).join("; ")}.`);
  }
  if (profileMissing.length) {
    parts.push(`Not on this computer: ${describeMissing(profileMissing)}. Choose a device under Modem devices and save.`);
  }
  parts.push(await applied(report));
  noteProfile(parts.join(" "), profileMissing.length || (report.invalid ?? []).length ? "warn" : undefined);
  log(`profile ${answer.loaded?.name ?? profiles.name ?? ""} loaded (${(report.changed ?? []).length} settings changed)`);
  await loadConfig();
  loadMemories();
  refreshStatus();
}

const MISSING_WORDS = {
  "audio.input": "capture device",
  "audio.output": "playback device",
  "ptt.port": "keying port",
  "ptt.device": "keying interface",
};

function describeMissing(missing) {
  return missing
    .map((m) => {
      const what = `${MISSING_WORDS[m.key] ?? m.key} ${m.name}`;
      return m.suggestion ? `${what} (${m.suggestion} has the same interface behind it)` : what;
    })
    .join(", ");
}

async function loadProfile(id) {
  const entry = profiles.list.find((p) => p.id === id);
  try {
    const answer = await call("profile.load", { id });
    await afterProfileLoad(answer, `Switched to ${entry?.name ?? id}.`);
  } catch (error) {
    noteProfile(`${entry?.name ?? id} was not loaded: ${error.message} Nothing was changed.`, "error");
    log(error.message, true);
    renderProfiles();
  }
}

/// A switch, a new profile or an import with unsaved changes on the current one asks
/// first: the changes are in the modem and would stay there, but the profile they
/// belong to would not have them.
function withProfileSaved(what, go) {
  $("profile-more").open = false;
  if (profiles.dirty !== true || !profiles.active) {
    go();
    return;
  }
  profileAfterPrompt = go;
  $("profile-prompt-text").textContent = `${profiles.name} has changes that are not saved to it. Save them before ${what}?`;
  $("profile-prompt").hidden = false;
  $("profile-prompt-save").focus();
}

async function answerProfilePrompt(choice) {
  $("profile-prompt").hidden = true;
  const go = profileAfterPrompt;
  profileAfterPrompt = null;
  if (choice === "cancel" || !go) {
    renderProfiles();
    return;
  }
  if (choice === "save") {
    try {
      applyProfiles(await call("profile.save", {}));
    } catch (error) {
      noteProfile(error.message, "error");
      renderProfiles();
      return;
    }
  }
  go();
}

async function saveProfile() {
  try {
    const answer = await call("profile.save", {});
    applyProfiles(answer);
    noteProfile(`Saved to ${answer.saved?.name ?? profiles.name} at ${new Date().toLocaleTimeString()}.`);
    log(`profile ${answer.saved?.name ?? ""} saved`);
  } catch (error) {
    noteProfile(error.message, "error");
    log(error.message, true);
  }
}

async function exportProfile() {
  $("profile-more").open = false;
  try {
    const answer = await call("profile.export", profiles.active ? { id: profiles.active } : {});
    const blob = new Blob([answer.text], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = answer.filename;
    document.body.append(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 10000);
    noteProfile(
      `${answer.filename} is being saved by the browser${answer.path ? `; the modem's own copy is ${answer.path}` : ""}.`,
    );
  } catch (error) {
    noteProfile(error.message, "error");
    log(error.message, true);
  }
}

async function importProfileFile(file) {
  let text;
  try {
    text = await file.text();
  } catch (error) {
    noteProfile(`${file.name} could not be read: ${error.message}`, "error");
    return;
  }
  let answer;
  try {
    answer = await call("profile.import", { text });
  } catch (error) {
    if (!/already exists/.test(error.message)) {
      noteProfile(`${file.name}: ${error.message}`, "error");
      log(error.message, true);
      return;
    }
    if (!window.confirm(`${error.message} Replace it with the file's?`)) return;
    try {
      answer = await call("profile.import", { text, replace: true });
    } catch (again) {
      noteProfile(`${file.name}: ${again.message}`, "error");
      return;
    }
  }
  applyProfiles(answer);
  const name = answer.imported?.name ?? file.name;
  const report = answer.report ?? {};
  const remarks = [];
  if ((report.unknown ?? []).length) remarks.push(`settings this version does not have: ${report.unknown.join(", ")}`);
  if ((report.invalid ?? []).length) remarks.push(`values that will be kept at the default: ${report.invalid.map((r) => r.key).join(", ")}`);
  if ((report.missing_hardware ?? []).length) remarks.push(`devices not on this computer: ${describeMissing(report.missing_hardware)}`);
  const written = answer.aether_version ? ` (written by Aether HF ${answer.aether_version})` : "";
  noteProfile(`Imported ${name}${written}.${remarks.length ? ` ${remarks.join("; ")}.` : ""}`, remarks.length ? "warn" : undefined);
  log(`profile ${name} imported`);
  if (window.confirm(`Load ${name} now?${remarks.length ? `\n\n${remarks.join(".\n")}.` : ""}`)) {
    withProfileSaved("switching", () => loadProfile(answer.imported.id));
  }
}

/// The devices the configuration names that this machine does not list: shown in the
/// device lists as what they are, and said under them, so a profile from another
/// computer — or a radio that is unplugged — is never mistaken for "system default".
function noteMissingDevices() {
  const missing = [];
  const check = (select, name, key, word) => {
    if (!name) return;
    if ([...select.options].some((o) => o.value === name)) return;
    const option = document.createElement("option");
    option.value = name;
    option.textContent = `${name} — not on this computer`;
    option.dataset.missing = "true";
    select.append(option);
    select.value = name;
    const hint = profileMissing.find((m) => m.key === key);
    missing.push({ key, name, suggestion: hint?.suggestion, word });
  };
  const ptt = liveConfig?.ptt ?? {};
  check($("dev-in"), liveConfig?.audio?.input, "audio.input");
  check($("dev-out"), liveConfig?.audio?.output, "audio.output");
  if (ptt.kind === "serial" || ptt.kind === "cat") check($("dev-ptt"), ptt.port, "ptt.port");
  if (ptt.kind === "cm108" && ptt.device) {
    const listed = [...$("dev-ptt").options].some((o) => o.value === `gpio:${ptt.device}`);
    if (!listed) missing.push({ key: "ptt.device", name: ptt.device });
  }
  const note = $("hardware-note");
  note.hidden = missing.length === 0;
  note.textContent = missing.length
    ? `Not on this computer: ${describeMissing(missing)}. The modem cannot use it; choose a device that is here and save.`
    : "";
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
    // CAT rather than RTS: the one USB serial port carries CI-V, so keying by command costs
    // nothing and reads the dial, which the rules need — RTS keying left an IC-7300 keying
    // well with no dial at all (ND1J, 2026-09-25)
    line: "cat",
    // 19200, CI-V's own rate: what Icom operators already use, and what the REMOTE jack runs at
    cat: { protocol: "icom", baud: 19200, civ: "94" },
    note: "Icom's USB port carries audio and CI-V: Aether keys by CAT command and reads the dial. In the rig's menu (MENU › SET › Connectors › CI-V) set CI-V USB Port to Unlink from [REMOTE] and CI-V USB Baud Rate to the rate here, 19200 — or put the rig's own rate here, such as 38400. The address is the rig's: IC-7300 94, IC-7610 98, IC-9700 A2, IC-705 A4.",
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
    name: "Manual — the devices and keying below, as you set them",
    match: null,
    ptt: null,
    line: null,
    note: "The fields below are what the modem uses. Pick a known interface above to have them filled in for it — you can change any of them afterwards.",
  },
];

/// The Manual entry: shown whenever the fields below are not exactly what a known interface
/// would fill in.
const MANUAL = PROFILES.findIndex((p) => p.id === "manual");

let devicesSeen = { devices: [], serial_ports: [], gpio_interfaces: [] };

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
  // A rebuild shows what was shown; it never fills anything in. It used to guess an
  // interface from the devices and apply it, which rewrote the keying under the operator:
  // an Icom's and a Yaesu's codecs are both "USB Audio CODEC", and a Yaesu keyed by CAT was
  // shown — and, on the next Save, set — as an Icom.
  showInterface(before === "" ? -1 : Number(before));
}

/// Whether the fields below are exactly what an interface would fill in: its devices, its
/// keying port's kind and line, and its CAT settings. Anything else is the operator's own
/// setup, which the list shows as Manual rather than as an interface it no longer is.
function interfaceMatches(profile) {
  if (!profile || profile.id === "manual") return false;
  const input = $("dev-in").value;
  if (profile.match && !(input && profile.match.test(input))) return false;
  const keying = $("dev-ptt").value;
  if (profile.ptt === "none") return keying === "";
  if (profile.ptt === "gpio") return keying.startsWith("gpio:");
  if (keying === "" || keying === "rigctld" || keying.startsWith("gpio:")) return false;
  if (profile.line && $("ptt-line").value !== profile.line) return false;
  if (profile.cat) {
    if ($("ptt-protocol").value !== profile.cat.protocol) return false;
    if (Number($("ptt-baud").value) !== profile.cat.baud) return false;
    if (profile.cat.civ && $("ptt-civ").value.trim().toUpperCase() !== profile.cat.civ) return false;
  }
  return true;
}

/// Show `index` when the fields are still what it fills in, and Manual otherwise.
function showInterface(index) {
  const shown = index >= 0 && interfaceMatches(PROFILES[index]) ? index : MANUAL;
  $("wz-profile").value = String(shown);
  $("wz-profile-note").textContent = PROFILES[shown]?.note ?? "";
  if (shown === MANUAL) noteDetectedInterface();
}

/// On Manual with no capture device chosen yet, say which known interfaces this machine
/// seems to have — a hint, never a choice made for the operator.
function noteDetectedInterface() {
  if ($("dev-in").value) return;
  const seen = PROFILES.filter(
    (p) => p.match && devicesSeen.devices.some((d) => p.match.test(d.name)),
  ).map((p) => p.name);
  if (seen.length === 0) return;
  $("wz-profile-note").textContent =
    `This machine has a device that looks like ${seen.join(" or ")}: pick yours above to fill the fields below in for it.`;
}

/// A field below changed by hand: the list says Manual unless it is still what the chosen
/// interface fills in.
function interfaceEdited() {
  const index = Number($("wz-profile").value);
  if (!interfaceMatches(PROFILES[index])) showInterface(MANUAL);
}

function applyProfile() {
  const profile = PROFILES[Number($("wz-profile").value)] ?? PROFILES[MANUAL];
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
  // how the port keys, as the interface is known to key: the operator picked this profile
  if (profile.ptt === "serial" && profile.line) select($("ptt-line"), profile.line);
  if (profile.cat) {
    select($("ptt-protocol"), profile.cat.protocol);
    $("ptt-baud").value = String(profile.cat.baud);
    if (profile.cat.civ) $("ptt-civ").value = profile.cat.civ;
  }
  showKeyingFields();
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
  const ok = numberWithin("host-port", 1024, 65535) && Boolean($("update-channel").value) && kissFormOk();
  showKissExposed();
  markStep(5, ok);
  return ok;
}

// ── the KISS port (ADR-0019) ────────────────────────────────────────
//
// APRS and packet programs connect to it as they would to VARA HF's; what it says here is
// the daemon's `status.kiss` (listening, the programs connected, their frames) and the form
// that sets `[kiss]`. A port that anyone on the network could reach is said out loud: it
// makes the station transmit and asks no password.

let kissExposedRunning = false;
let kissClientsShown = "";
// the configured rung, held until the mode table arrives with the option for it
let kissRungWanted = "1";

/// The host part of `host:port`, an IPv6 host's brackets taken off.
function hostOf(address) {
  const text = String(address ?? "");
  const cut = text.lastIndexOf(":");
  const host = cut >= 0 ? text.slice(0, cut) : text;
  return host.startsWith("[") && host.endsWith("]") ? host.slice(1, -1) : host;
}

/// `host:port` as the daemon parses it: an IPv6 host goes in brackets.
function joinAddress(host, port) {
  return host.includes(":") ? `[${host}]:${port}` : `${host}:${port}`;
}

/// Whether an address lets only this computer in.
function loopbackHost(host) {
  const h = String(host ?? "").trim().toLowerCase();
  return h === "localhost" || h === "::1" || h.startsWith("127.");
}

/// The KISS rung list: the running ladder, with the two rungs every station hears marked.
function fillKissRungs() {
  const choice = $("kiss-rung");
  if (choice.options.length === modeTable.length && modeTable.length > 0) return;
  const before = choice.options.length > 1 ? choice.value : kissRungWanted;
  choice.replaceChildren();
  for (const mode of modeTable) {
    const option = document.createElement("option");
    option.value = String(mode.index);
    // the tone floor's first two rungs are the same frames in both bandwidths (ADR-0013)
    option.textContent = mode.index < 2
      ? `${mode.index} — ${mode.name} · heard at 500 and 2300 Hz`
      : `${mode.index} — ${mode.name} · this bandwidth only`;
    choice.append(option);
  }
  select(choice, before);
}

function kissFormOk() {
  if (!$("kiss-enabled").checked) return true;
  if ($("kiss-host").value.trim() === "" || !numberWithin("kiss-port", 1024, 65535)) return false;
  // the host programs' interface holds its port and the next one up
  const port = Number($("kiss-port").value);
  const host = Number($("host-port").value);
  return !($("host-enabled").checked && (port === host || port === host + 1));
}

function showKissExposed() {
  const typed = $("kiss-enabled").checked && !loopbackHost($("kiss-host").value);
  $("kiss-exposed").hidden = !(typed || kissExposedRunning);
}

function applyKiss(kiss, host, datagrams) {
  const k = kiss ?? { enabled: false, listening: false, clients: [] };
  const clients = k.clients ?? [];
  const count = clients.length;
  const programs = `${count} program${count === 1 ? "" : "s"}`;
  let state;
  if (!k.enabled) state = "off";
  else if (!k.listening) state = `not listening: ${k.error ?? "starting"}`;
  else state = count === 0 ? `listening on ${k.address}` : `${programs} connected on ${k.address}`;
  if (k.listening && k.paused) state += ` · holding frames: ${k.paused}`;
  const line = $("kiss-state");
  line.textContent = state;
  line.dataset.state = k.enabled && !k.listening ? "error" : k.listening && k.paused ? "warn" : "ok";
  $("btn-kiss-disconnect").disabled = count === 0;
  renderKissClients(clients);
  kissExposedRunning = k.listening === true && k.exposed === true;
  showKissExposed();

  $("d-kiss").textContent = !k.enabled
    ? "off"
    : !k.listening
      ? "not listening"
      : count > 0
        ? `${programs}`
        : "listening";
  const waiting = datagrams?.queued ?? 0;
  $("d-kiss-sub").textContent = k.enabled
    ? `${k.frames_in ?? 0} in · ${k.frames_out ?? 0} out · ${waiting} waiting`
    : "turn on KISS programs in Setup";

  // the header: which programs are using this station now
  const using = [];
  if (host?.connected) using.push("host program");
  if (count > 0) using.push(`${count} KISS`);
  const chip = $("apps-chip");
  chip.hidden = using.length === 0;
  chip.textContent = using.join(" · ");
  const who = clients.map((c) => `${c.app} (${c.peer})`);
  chip.title = [
    host?.connected ? "A host program is attached on the VARA-compatible interface" : "",
    who.length ? `KISS: ${who.join(", ")}` : "",
  ].filter(Boolean).join(". ") || "Programs using this station now";
}

function renderKissClients(clients) {
  // rebuilt only when something changed, so a Disconnect button is not replaced under the
  // pointer between a press and its release
  const key = JSON.stringify(clients);
  if (key === kissClientsShown) return;
  kissClientsShown = key;
  const list = $("kiss-clients");
  list.hidden = clients.length === 0;
  list.replaceChildren(
    ...clients.map((c) => {
      const item = document.createElement("li");
      const since = c.since_ms
        ? new Date(c.since_ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
        : "";
      const text = document.createElement("span");
      const dropped = c.dropped ? `, ${c.dropped} dropped` : "";
      text.textContent = `${c.app} · ${c.peer} · since ${since} · ${c.frames_in} in, ${c.frames_out} out${dropped}`;
      text.title =
        "What the program seems to be (a guess from what it sends), where it connected from, when, and its frames: sent to the air, handed to it, dropped";
      const button = document.createElement("button");
      button.className = "small ghost";
      button.textContent = "Disconnect";
      button.title = `Close this program's connection (${c.peer}); it may connect again`;
      button.addEventListener("click", () => kissDisconnect(c.id));
      item.append(text, button);
      return item;
    }),
  );
}

async function kissDisconnect(client) {
  try {
    const result = await call("kiss.disconnect", client == null ? {} : { client });
    const closed = result.disconnected ?? 0;
    log(`KISS: ${closed} connection${closed === 1 ? "" : "s"} closed`, false, "kiss");
  } catch (error) {
    log(`KISS: ${error.message}`, true, "kiss");
  }
  refreshStatus();
}

async function wizardSave() {
  $("wz-save-note").textContent = "";
  const call_ = $("wz-call").value.trim().toUpperCase();
  if (!call_) {
    $("wz-save-note").textContent = "A callsign is required.";
    return;
  }
  const profile = PROFILES[Number($("wz-profile").value)] ?? PROFILES[MANUAL];
  // the fields are what is saved: an interface fills them in when it is picked, and never
  // again — a VOX interface shown in the list used to save "no keying" over a CAT port set
  // below it
  const changes = formChanges();
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

// the busy detector's threshold over the floor, from the configuration, for the chart's line
let busyThresholdDb = 6;

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
  keyingSummary();
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

// A drive check is real bursts, not a tone: measured at the sound card the waveform's peaks
// sit 5.8 dB (floor mode) to 7.0 dB (fastest) above a tone of the same average power, and it
// is peaks the rig's ALC answers to. Setting drive on the tone leaves the modem that far
// into limiting on traffic.
const DRIVE_BURSTS = 4;
// the daemon's own rhythm: six seconds of waveform, five of silence, so a hand on the drive
// control has time to read the meter, move, and see the next burst land
const DRIVE_BURST_S = 6;
const DRIVE_GAP_S = 5;
let driveTimer = null;

function driveButton(sending) {
  $("wz-drive").textContent = sending ? "Stop the bursts" : `Set drive, ${DRIVE_BURSTS} bursts`;
  $("wz-drive").setAttribute("aria-pressed", String(sending));
  clearTimeout(driveTimer);
  // the whole check plus a little; the daemon stops on its own either way
  const total = DRIVE_BURSTS * DRIVE_BURST_S + (DRIVE_BURSTS - 1) * DRIVE_GAP_S + 4;
  driveTimer = sending ? setTimeout(() => driveButton(false), total * 1000) : null;
}

async function toggleDrive() {
  if ($("wz-drive").getAttribute("aria-pressed") === "true") {
    driveButton(false);
    try {
      await call("drive.set", { bursts: 0 });
      $("wz-tx-note").textContent = "Drive check stopped.";
    } catch (error) {
      $("wz-tx-note").textContent = error.message;
    }
    return;
  }
  $("wz-tx-note").textContent = "Drive check…";
  try {
    await call("drive.set", { bursts: DRIVE_BURSTS });
    $("wz-tx-note").textContent = `${DRIVE_BURSTS} bursts of ${DRIVE_BURST_S} s, ${DRIVE_GAP_S} s apart — watch the ALC, back the level off until it barely moves.`;
    log(`drive check, ${DRIVE_BURSTS} bursts at transmit level ${$("tx-level-reading").textContent}`);
    driveButton(true);
    setKeyingOpen(true);
  } catch (error) {
    $("wz-tx-note").textContent = error.message;
    log(error.message, true);
  }
}

// The receiver's passband against what the modem needs. The modem measures the passband
// from the noise between signals, so a radio filter set narrower than the signal — the
// mismatch that carries only the middle of every burst and looks like a dead band — is
// caught without asking the rig. Warn only when it is clearly narrower, and never before
// the modem has heard enough noise to say (a null reading).
function applyPassband(measured, needed) {
  const note = $("rx-passband-note");
  if (!note) return;
  if (typeof measured !== "number" || typeof needed !== "number" || measured >= needed - 400) {
    note.hidden = true;
    return;
  }
  const round = (hz) => Math.round(hz / 50) * 50;
  note.textContent =
    `The radio's receive filter looks about ${round(measured)} Hz wide, but the modem's ` +
    `signal needs about ${Math.round(needed)} Hz — it is carrying only the middle of every ` +
    `burst. Widen the radio's filter (roofing or DSP width) to pass the whole signal, or set ` +
    `the modem to 500 Hz in Setup.`;
  note.hidden = false;
}

// The peak the modem actually handed the sound card on its last transmission. This is the
// number the operator cannot read off the rig: an ALC meter shows that the rig is limiting,
// not by how much, and nothing on the radio shows what arrived before its own gain stages.
function applyTxPeak(dbfs) {
  const box = $("tx-peak");
  if (!box) return;
  if (typeof dbfs !== "number" || !isFinite(dbfs)) {
    box.textContent = "—";
    box.classList.remove("warn", "bad");
    return;
  }
  box.textContent = `${dbfs >= 0 ? "" : "−"}${Math.abs(dbfs).toFixed(1)} dB`;
  box.classList.toggle("bad", dbfs > -1);
  box.classList.toggle("warn", dbfs > -3 && dbfs <= -1);
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

// ── the rules ───────────────────────────────────────────────────────
//
// The modem judges every transmission against its regulatory profile before the radio is
// keyed (ADR-0018). The panel shows the verdict on this station's widest transmission at the
// dial it is on — LEGAL, WARNING or TX BLOCKED — in the header on every tab and beside the
// dial on the Session tab, with the reasoning one click away; what Setup still needs, in a
// banner; and on the Diagnostics tab everything the policy knows: the situation, the ceiling
// it holds the link to, the last decision the gate made and the dials where each waveform
// fits. The panel decides nothing itself: every word here is the modem's.

let regulatory = null;
let regRefreshTimer = null;

const VERDICT_WORD = {
  legal: "LEGAL",
  warning: "WARNING",
  blocked: "TX BLOCKED",
  none: "NO RULES",
  unknown: "—",
};

function verdictOf(reg) {
  if (!reg) return "unknown";
  if (reg.policy === "none") return "none";
  return reg.indicator?.verdict ?? "unknown";
}

function regSummary(reg) {
  if (!reg) return "";
  if (reg.policy === "none") return "No regulatory profile: you check every transmission yourself.";
  return reg.indicator?.summary ?? reg.error ?? "";
}

/// "7 101.500 kHz": kilohertz with a thin space between the thousands.
function khz(hz) {
  if (typeof hz !== "number") return "—";
  const [whole, frac] = (hz / 1000).toFixed(3).split(".");
  return `${whole.replace(/\B(?=(\d{3})+(?!\d))/g, " ")}.${frac} kHz`;
}

/// The e-CFR page of a Part 97 rule, from its citation: "§97.305(c)" → section 97.305.
function ecfrUrl(rule) {
  const m = /97\.(\d+)/.exec(rule ?? "");
  return m ? `https://www.ecfr.gov/current/title-47/section-97.${m[1]}` : null;
}

const CONTROL_WORD = { local: "local", remote: "remote", automatic: "automatic" };
const LICENSE_WORD = {
  novice: "Novice",
  technician: "Technician",
  general: "General",
  advanced: "Advanced",
  extra: "Amateur Extra",
};

/// What Setup (or the dial) still needs before this station may transmit, in a sentence, or
/// nothing when it needs nothing.
function regNeeds(reg) {
  if (!reg) return "";
  if (reg.policy === "unset") {
    return "Transmitting is blocked until you say which rules this station operates under — Setup, step 1.";
  }
  if (reg.policy === "broken") {
    return `The regulatory profile could not be read (${reg.error ?? "unknown error"}); nothing is transmitted until it can be.`;
  }
  switch (reg.indicator?.code) {
    case "no_control":
      return "Transmitting is blocked until you say how the station is controlled — local, remote or automatic — Setup, step 1.";
    case "no_license":
      return "Transmitting is blocked until you set your license class — Setup, step 1.";
    case "no_sideband":
      return "Transmitting is blocked until you set the sideband the radio transmits on — Setup, step 1.";
    case "region":
      return `${reg.indicator.summary}. ${reg.indicator.detail}`;
    case "no_dial":
      return canTune
        ? "The dial frequency is not known yet: the radio has not reported it. Nothing is transmitted until it does."
        : "The dial frequency is not known: this radio cannot report it, so say where it is on the Session tab (Dial is at) before transmitting.";
    default:
      return "";
  }
}

function applyRegulatory(reg) {
  regulatory = reg ?? null;
  const verdict = verdictOf(regulatory);
  const summary = regSummary(regulatory);
  const tone = verdict === "unknown" || verdict === "none" ? "" : verdict;

  const badge = $("reg-badge");
  badge.hidden = regulatory === null;
  badge.dataset.verdict = verdict;
  $("reg-badge-text").textContent = VERDICT_WORD[verdict];
  const hint = summary ? `${summary} — click for the reasoning` : "Whether this station may transmit here; click for the reasoning";
  if (badge.title !== hint) badge.title = hint;
  $("reg-badge").setAttribute("aria-label", `Rules: ${VERDICT_WORD[verdict]}. ${summary}`);

  const chip = $("dial-hero-reg");
  chip.hidden = regulatory === null;
  chip.dataset.verdict = verdict;
  $("dial-hero-reg-badge").dataset.verdict = verdict;
  $("dial-hero-reg-badge").textContent = VERDICT_WORD[verdict];
  $("dial-hero-reg-text").textContent = summary.replace(/^(FCC: |TX BLOCKED: |FCC warning: )/, "");

  const needs = regNeeds(regulatory);
  $("reg-banner").hidden = !needs;
  $("reg-banner-text").textContent = needs;
  $("btn-reg-banner-setup").hidden = regulatory?.indicator?.code === "no_dial";

  $("reg-detail").style.setProperty("--tone", tone ? `var(--status-${tone === "legal" ? "link" : tone === "blocked" ? "error" : "warning"})` : "var(--border-strong)");
  if (!$("reg-detail").hidden) renderRegDetail();
  if (panelShown("diagnostics")) renderRegCard();
  checkRulesStep();
}

// A decision the gate made, from the `regulatory` event: logged, and the badge refreshed a
// moment later (the status carries the new ceiling and the last decision).
function onRegulatory(d) {
  const blocked = d.verdict === "blocked";
  const rf =
    typeof d.rf_low_hz === "number" && typeof d.rf_high_hz === "number"
      ? ` · on the air ${khz(d.rf_low_hz)}–${khz(d.rf_high_hz)}`
      : "";
  log(
    `${blocked ? "refused" : "permitted"} ${d.what}: ${d.summary}${rf}${d.detail ? `\n${d.detail}` : ""}`,
    blocked ? "warn" : "info",
    "rules",
  );
  clearTimeout(regRefreshTimer);
  regRefreshTimer = setTimeout(refreshStatus, 300);
}

// ── the reasoning, dropped from the header ──

function regFact(list, name, value, title, href = null) {
  const dt = document.createElement("dt");
  dt.textContent = name;
  const dd = document.createElement("dd");
  dd.title = title;
  if (href) {
    const a = document.createElement("a");
    a.href = href;
    a.target = "_blank";
    a.rel = "noopener";
    a.textContent = value;
    a.title = `${title} — opens the e-CFR`;
    dd.append(a);
  } else {
    dd.textContent = value;
  }
  list.append(dt, dd);
}

function describeDial(d) {
  if (typeof d.dial_hz !== "number") return "not known";
  const source = d.dial_source === "declared" ? "declared by you" : "read from the radio";
  return `${khz(d.dial_hz)} (${source}), ${(d.sideband ?? "?").toUpperCase()}`;
}

function renderRegDetail() {
  const reg = regulatory;
  const d = reg?.indicator ?? null;
  const verdict = verdictOf(reg);
  $("reg-detail-verdict").dataset.verdict = verdict;
  $("reg-detail-verdict").textContent = VERDICT_WORD[verdict];
  $("reg-detail-summary").textContent = regSummary(reg) || "The modem has not said yet.";
  $("reg-detail-text").textContent =
    reg?.policy === "none"
      ? "The operator chose no regulatory profile: Aether makes no regulatory checks, and the control operator is responsible for every transmission."
      : (d?.detail ?? "");
  const facts = $("reg-detail-facts");
  facts.replaceChildren();
  if (d && reg.policy === "rules") {
    if (typeof d.rf_low_hz === "number") {
      regFact(facts, "On the air", `${khz(d.rf_low_hz)} – ${khz(d.rf_high_hz)}`, "The RF range the transmission covers: the dial and the audio it occupies, on this sideband");
    }
    regFact(facts, "Occupies", `${Math.round(d.bandwidth_hz)} Hz of audio (${Math.round(d.audio_low_hz)}–${Math.round(d.audio_high_hz)} Hz)`, "Measured from the waveform, under the wider reading of §97.3(a)(8)");
    regFact(facts, "Dial", describeDial(d), "The dial the decision used, and where it came from");
    if (d.band) regFact(facts, "Band", d.band, "The amateur band the signal is in");
    if (d.segment) {
      regFact(facts, "Segment", `${khz(d.segment.low_hz)} – ${khz(d.segment.high_hz)}${d.segment_rule ? ` (${d.segment_rule})` : ""}`, "The segment the whole signal must stay inside, with the margin kept at its edges", ecfrUrl(d.segment_rule));
    }
    if (d.automatic_segment) {
      regFact(facts, "Automatic sub-band", `${khz(d.automatic_segment.low_hz)} – ${khz(d.automatic_segment.high_hz)}`, "The §97.221(b) segment an automatically controlled station is inside");
    }
    regFact(facts, "Control", CONTROL_WORD[d.control] ?? "not set", "How the station is controlled (§97.109)");
    regFact(facts, "License", LICENSE_WORD[d.license] ?? "not set", "The control operator's class (§97.301)");
    regFact(facts, "Margin", `${Math.round(d.margin_hz)} Hz at each edge`, "Room kept between the signal's edges and the segment's (Setup step 4)");
    if (d.rule) regFact(facts, "Rule", d.rule, "The rule that decided", ecfrUrl(d.rule));
    const ceiling = reg.ceiling;
    if (ceiling && typeof ceiling.rung === "number" && ceiling.rung < ceiling.of - 1) {
      regFact(facts, "Link held to", `rungs 0–${ceiling.rung}${ceiling.name ? ` (${ceiling.name})` : ""} of ${ceiling.of}`, "The fastest rung the rules allow here: the link never climbs past it");
    }
  }
  const extra = [d?.guidance, ...(d?.notes ?? [])].filter(Boolean).join(" ");
  $("reg-detail-guidance").textContent = extra;
  $("reg-detail-guidance").hidden = !extra;
}

function openRegDetail() {
  const panel = $("reg-detail");
  const header = document.querySelector("header.bar");
  panel.style.top = `${Math.round(header.getBoundingClientRect().bottom + 6)}px`;
  renderRegDetail();
  panel.hidden = false;
  for (const id of ["reg-badge", "dial-hero-reg"]) $(id).setAttribute("aria-expanded", "true");
}

function closeRegDetail() {
  $("reg-detail").hidden = true;
  for (const id of ["reg-badge", "dial-hero-reg"]) $(id).setAttribute("aria-expanded", "false");
}

function toggleRegDetail() {
  if ($("reg-detail").hidden) openRegDetail();
  else closeRegDetail();
}

function showRulesInSetup() {
  closeRegDetail();
  selectTab($("tab-setup"));
  $("step-1").scrollIntoView({ behavior: "smooth", block: "start" });
  const first = ["reg-profile", "reg-control", "reg-license"].find((id) => !$(id).value) ?? "reg-profile";
  $(first).focus({ preventScroll: true });
}

function showRulesInDiagnostics() {
  closeRegDetail();
  selectTab($("tab-diagnostics"));
  $("reg-card").scrollIntoView({ behavior: "smooth", block: "start" });
}

// ── the Diagnostics card ──

function safeRange(entry) {
  if (!entry) return "—";
  if (entry.dial_low_hz === entry.dial_high_hz) return `${khz(entry.dial_low_hz)} (channel)`;
  return `${khz(entry.dial_low_hz)} – ${khz(entry.dial_high_hz)}`;
}

function renderRegCard() {
  const reg = regulatory;
  const verdict = verdictOf(reg);
  $("reg-card-verdict").dataset.verdict = verdict;
  $("reg-card-verdict").textContent = VERDICT_WORD[verdict];
  const profile = reg?.profile ?? null;
  $("reg-card-source").textContent = profile
    ? `${profile.name} · rules as of ${profile.rules_as_of}`
    : reg?.policy === "none"
      ? "no profile: the operator checks every transmission"
      : reg?.policy === "unset"
        ? "no profile chosen yet"
        : "";
  const d = reg?.indicator ?? null;
  $("reg-card-summary").textContent = [regSummary(reg), d?.detail].filter(Boolean).join(" — ");

  const s = reg?.situation ?? {};
  const situation = $("reg-situation");
  situation.replaceChildren();
  if (reg) {
    const settings = liveConfig?.regulatory ?? {};
    regFact(situation, "Rules", profile ? profile.name : reg.policy === "none" ? "none" : "not chosen", "The regulatory profile in force (Setup step 1)");
    regFact(situation, "Control", CONTROL_WORD[s.control] ?? "not set", "How the station is controlled (§97.109), from Setup step 1 — never guessed from the network");
    regFact(situation, "License", LICENSE_WORD[s.license] ?? "not set", "The control operator's class (§97.301)");
    regFact(situation, "Dial", typeof s.dial_hz === "number" ? `${khz(s.dial_hz)} (${s.dial_source === "declared" ? "declared" : "from the radio"})` : "not known", "The dial the decisions use: read from the radio over CAT or rigctld, or declared on the Session tab");
    regFact(situation, "Sideband", (s.sideband ?? "not set").toUpperCase(), "The sideband the radio transmits on (Setup step 1)");
    regFact(situation, "Direction", reg.direction ?? "—", "Who began the exchange a transmission would belong to now: originate (this station), respond (another station called), operator (a test)");
    regFact(situation, "Margin", `${Math.round(s.margin_hz ?? settings.edge_margin_hz ?? 0)} Hz at each segment edge`, "Room kept between the signal's edges and a segment's (Setup step 4)");
    regFact(situation, "ITU region", String(s.itu_region ?? "—"), "The region whose tables apply");
  }

  const limits = $("reg-limits");
  limits.replaceChildren();
  if (reg?.policy === "rules") {
    const c = reg.ceiling;
    const held = c && typeof c.rung === "number" && c.rung < c.of - 1;
    regFact(
      limits,
      "Link ceiling",
      !c ? "not computed yet" : c.rung === null ? "no rung allowed here" : held ? `rung ${c.rung}${c.name ? ` (${c.name})` : ""} of ${c.of - 1}` : "every rung allowed",
      "The fastest rung the rules allow at this dial: the link's rate control and its negotiation never go past it",
    );
    if (c?.limit) regFact(limits, "Held by", c.limit.summary, c.limit.detail ?? "Why the link is held there", ecfrUrl(c.limit.rule));
    const widest = reg.occupied?.widest;
    if (widest) {
      const [lo, hi] = widest.audio.spectral;
      regFact(limits, "Widest waveform", `rung ${widest.rung}: ${Math.round(Math.max(hi, widest.audio.power[1]) - Math.min(lo, widest.audio.power[0]))} Hz`, "The widest thing this station sends at its fastest mode, measured");
    }
    const floor = reg.occupied?.floor;
    if (floor) {
      regFact(limits, "Tone floor", `${Math.round(floor.audio.spectral[1] - floor.audio.spectral[0])} Hz`, "The narrowest data this station sends: the tone floor, measured");
    }
    const last = reg.last;
    if (last?.decision) {
      const ago = formatDuration(last.age_s ?? 0);
      regFact(limits, "Last decision", `${ago} ago: ${last.decision.verdict === "blocked" ? "refused" : "permitted"} — ${last.decision.what}`, last.decision.summary);
    }
  }

  const body = $("reg-safe");
  body.replaceChildren();
  const rows = new Map();
  for (const [key, list] of [["widest", reg?.safe_dials?.widest ?? []], ["floor", reg?.safe_dials?.floor ?? []]]) {
    for (const entry of list) {
      const id = `${entry.band}|${entry.segment.low_hz}|${entry.segment.high_hz}`;
      if (!rows.has(id)) rows.set(id, { band: entry.band, segment: entry.segment, rule: entry.rule });
      rows.get(id)[key] = entry;
    }
  }
  for (const row of [...rows.values()].sort((a, b) => a.segment.low_hz - b.segment.low_hz)) {
    const tr = document.createElement("tr");
    const cell = (text, className = "", title = "", href = null) => {
      const td = document.createElement("td");
      if (className) td.className = className;
      if (title) td.title = title;
      if (href) {
        const a = document.createElement("a");
        a.href = href;
        a.target = "_blank";
        a.rel = "noopener";
        a.textContent = text;
        td.append(a);
      } else {
        td.textContent = text;
      }
      tr.append(td);
    };
    cell(row.band);
    cell(`${khz(row.segment.low_hz)} – ${khz(row.segment.high_hz)}`, "num");
    cell(safeRange(row.widest), "num", "Dial range for the widest waveform");
    cell(safeRange(row.floor), "num", "Dial range for the tone floor");
    cell(row.rule, "", "The rule the segment comes from", ecfrUrl(row.rule));
    body.append(tr);
  }
  $("reg-safe-wrap").hidden = rows.size === 0;
}

// ── a dial the radio cannot report ──

function applyDeclaredDial(status) {
  const reg = status.regulatory ?? null;
  const declared = liveConfig?.regulatory?.dial_hz ?? null;
  // a radio that reports its dial is always taken at its word: a declared one is only for
  // a radio that cannot (serial-line, CM108 or VOX keying)
  const row = $("declared-row");
  row.hidden = status.can_tune === true || !(reg?.policy === "rules" || declared);
  if (row.hidden) return;
  const input = $("declared-dial");
  if (document.activeElement !== input && declared && !input.value) {
    input.value = (declared / 1e6).toFixed(declared % 1000 === 0 ? 3 : 4);
  }
  $("btn-declared-clear").disabled = !declared;
  $("declared-note").textContent = declared
    ? `The rules place your signal from ${khz(declared)}; set it again whenever you move the dial.`
    : "This radio cannot report its dial: say where it is before transmitting.";
}

async function setDeclaredDial(clear = false) {
  const hz = clear ? null : parseMhz($("declared-dial").value);
  if (!clear && !hz) {
    $("declared-note").textContent = "A frequency in MHz is needed, such as 14.105.";
    $("declared-dial").focus();
    return;
  }
  try {
    await call("config.set", { "regulatory.dial_hz": hz });
    if (liveConfig) liveConfig.regulatory = { ...(liveConfig.regulatory ?? {}), dial_hz: hz };
    if (clear) $("declared-dial").value = "";
    log(clear ? "the declared dial is forgotten" : `the dial is declared at ${khz(hz)}`, false, "rules");
    refreshStatus();
  } catch (error) {
    $("declared-note").textContent = error.message;
    log(error.message, true, "rules");
  }
}

// Step 1 is done when the callsign is plausible and the rules are said: a profile — or "none"
// — and, under a profile, the control and the license class. Until then it is marked.
function rulesChosen() {
  const profile = $("reg-profile").value;
  if (profile === "none") return true;
  return Boolean(profile && $("reg-control").value && $("reg-license").value);
}

function checkRulesStep() {
  const call_ = $("wz-call").value.trim().toUpperCase();
  const plausible = /^[A-Z0-9/-]{1,9}$/.test(call_);
  const chosen = rulesChosen();
  markStep(1, plausible && chosen);
  $("step-1").dataset.needs = String(!chosen);
}

function wireRules() {
  $("reg-badge").addEventListener("click", toggleRegDetail);
  $("dial-hero-reg").addEventListener("click", toggleRegDetail);
  $("btn-reg-close").addEventListener("click", closeRegDetail);
  $("btn-reg-setup").addEventListener("click", showRulesInSetup);
  $("btn-reg-banner-setup").addEventListener("click", showRulesInSetup);
  $("btn-reg-diagnostics").addEventListener("click", showRulesInDiagnostics);
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !$("reg-detail").hidden) closeRegDetail();
  });
  document.addEventListener("click", (event) => {
    const panel = $("reg-detail");
    if (panel.hidden) return;
    if (panel.contains(event.target) || $("reg-badge").contains(event.target) || $("dial-hero-reg").contains(event.target)) return;
    closeRegDetail();
  });
  window.addEventListener("resize", () => {
    if (!$("reg-detail").hidden) openRegDetail();
  });
  for (const id of ["reg-profile", "reg-control", "reg-license", "reg-sideband"]) {
    $(id).addEventListener("change", checkRulesStep);
  }
  $("btn-declared-set").addEventListener("click", () => setDeclaredDial(false));
  $("btn-declared-clear").addEventListener("click", () => setDeclaredDial(true));
  $("declared-dial").addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      setDeclaredDial(false);
    }
  });
}

// ── keying and drive: a card that folds, with its numbers in its head ──
//
// The card sat at the foot of the Session tab and was found by scrolling. It is now under
// the call row, folded by default, and its head always shows the transmit level, the last
// transmission's peak, how the radio is keyed and the three buttons — so a drive check is
// one click from anywhere on the tab, and pressing Set drive opens the card on the slider.

const KEYING_OPEN_KEY = "aether.keying.open";

function setKeyingOpen(open, remember = true) {
  $("btn-keying-toggle").setAttribute("aria-expanded", String(open));
  $("keying-body").hidden = !open;
  $("keying-card").dataset.open = String(open);
  if (!remember) return;
  try {
    localStorage.setItem(KEYING_OPEN_KEY, open ? "1" : "0");
  } catch {
    // the choice holds for this page only
  }
}

function keyingSummary() {
  const level = $("tx-level-reading").textContent;
  const peak = $("tx-peak").textContent;
  const keyed = lastStatus?.ptt ? ` · keyed by ${lastStatus.ptt}` : "";
  $("keying-summary").textContent = `level ${level} · last peak ${peak}${keyed}`;
}

function wireKeying() {
  let open = false;
  try {
    open = localStorage.getItem(KEYING_OPEN_KEY) === "1";
  } catch {
    // nothing kept
  }
  setKeyingOpen(open, false);
  $("btn-keying-toggle").addEventListener("click", () => {
    setKeyingOpen($("btn-keying-toggle").getAttribute("aria-expanded") !== "true");
  });
  keyingSummary();
}

// ── the log ─────────────────────────────────────────────────────────
//
// One entry a row: the time, where it came from (a tag: the panel, the session's state, the
// modem's own names — probe, test, audio — or the rules), and the words. Problems carry their
// colour; filters narrow it to problems, the rules' decisions or the sessions; the view
// follows new entries unless the reader has scrolled up to read.

const LOG_KEEP = 500;
const SESSION_TAGS = new Set(["state", "probe", "probed", "test", "sent", "session", "connect", "call", "disc", "role", "ladder"]);
let logFilter = "all";
let logFollow = true;

function logShows(entry) {
  switch (logFilter) {
    case "problems":
      return entry.dataset.level === "error" || entry.dataset.level === "warn";
    case "rules":
      return entry.dataset.tag === "rules";
    case "session":
      return SESSION_TAGS.has(entry.dataset.tag);
    default:
      return true;
  }
}

function countLog() {
  const pane = $("log");
  const total = pane.childElementCount;
  const shown = logFilter === "all" ? total : [...pane.children].filter((e) => !e.hidden).length;
  $("log-count").textContent =
    logFilter === "all" ? `${total} ${total === 1 ? "entry" : "entries"}` : `${shown} of ${total} entries`;
}

function showLogFollow() {
  const mark = $("log-live");
  mark.dataset.live = String(logFollow);
  mark.textContent = logFollow ? "following" : "held — scroll down to follow";
}

/// Add an entry. `level` is true for an error (as it always was), or "warn", "error", "info";
/// `tag` says where it came from.
function log(message, level = false, tag = "panel") {
  const pane = $("log");
  const entry = document.createElement("div");
  entry.className = "log-entry";
  entry.dataset.level = level === true ? "error" : typeof level === "string" ? level : "info";
  entry.dataset.tag = tag;
  const when = document.createElement("span");
  when.className = "when";
  when.textContent = clock(Date.now());
  const label = document.createElement("span");
  label.className = "log-tag";
  label.textContent = tag;
  label.title = `From: ${tag}`;
  const text = document.createElement("span");
  text.className = "log-text";
  text.textContent = message;
  entry.append(when, label, text);
  entry.hidden = !logShows(entry);
  pane.append(entry);
  while (pane.childElementCount > LOG_KEEP) pane.firstElementChild.remove();
  if (logFollow) pane.scrollTop = pane.scrollHeight;
  countLog();
}

function setLogFilter(filter) {
  logFilter = filter;
  for (const chip of document.querySelectorAll(".log-toolbar .chip")) {
    chip.setAttribute("aria-pressed", String(chip.dataset.filter === filter));
  }
  for (const entry of $("log").children) entry.hidden = !logShows(entry);
  logFollow = true;
  $("log").scrollTop = $("log").scrollHeight;
  showLogFollow();
  countLog();
}

function wireLog() {
  for (const chip of document.querySelectorAll(".log-toolbar .chip")) {
    chip.addEventListener("click", () => setLogFilter(chip.dataset.filter));
  }
  const pane = $("log");
  pane.addEventListener("scroll", () => {
    const atBottom = pane.scrollTop + pane.clientHeight >= pane.scrollHeight - 4;
    if (atBottom !== logFollow) {
      logFollow = atBottom;
      showLogFollow();
    }
  });
  $("btn-clear-log").addEventListener("click", () => {
    pane.replaceChildren();
    countLog();
  });
  showLogFollow();
  countLog();
}

// ── Help / About: the version and its updates ───────────────────────
//
// Under the desktop shell the About card reads the updater's own view — the version running,
// the channel, what the last check found and how far an install has got — and Check for
// Updates asks the shell's updater, the one the Help menu uses, whose window then shows what
// it found and installs it on a yes. The button is off while a check, a download or an
// install is under way. A browser has no updater: the card says so and links the releases.

const RELEASES_URL = "https://github.com/KK4ODA/aether-hf/releases";
let updateView = null;
let updateAnswerTimer = null;

function describeUpdate(view) {
  const percent = view.total ? ` — ${Math.round((100 * view.downloaded) / view.total)} %` : "…";
  switch (view.phase) {
    case "checking":
      return ["busy", "Checking for updates…", null];
    case "up-to-date":
      return view.checked === false
        ? ["idle", "Not checked yet since Aether HF started.", null]
        : ["current", "Up to date: this is the newest version on the channel.", view.current];
    case "no-stable-yet":
      return ["current", "No stable release yet: this beta is the newest. Choose betas in Setup step 5 to be offered the next one.", view.current];
    case "available":
      return ["available", `Version ${view.version} is available — the updates window has its notes and installs it.`, view.version];
    case "downloading":
      return ["busy", `Downloading ${view.version}${percent}`, view.version];
    case "installing":
      return ["busy", `Installing ${view.version}: the modem stops, and Aether HF starts again on it.`, view.version];
    case "restart-required":
      return ["available", `Installed ${view.version}: restart Aether HF to run it.`, view.version];
    case "complete":
      return ["complete", `Updated to ${view.version} from ${view.from}.`, view.version];
    case "incomplete":
      return ["error", `The update to ${view.version} did not finish; this is still the version before it.`, null];
    default:
      return ["error", view.message ?? "The updater reported a problem.", null];
  }
}

function applyUpdateView(view) {
  if (!view) return;
  updateView = view;
  clearTimeout(updateAnswerTimer);
  const [state, text, latest] = describeUpdate(view);
  $("about-update").dataset.state = state;
  $("about-update-status").textContent = text;
  $("about-version").textContent = view.current ?? "—";
  $("about-channel").textContent = view.channel ?? "—";
  $("about-latest").textContent = latest ?? "—";
  const busy = ["checking", "downloading", "installing"].includes(view.phase);
  $("btn-check-updates").disabled = busy;
}

function wireAbout() {
  const events = shellEvents();
  const button = $("btn-check-updates");
  if (events?.listen) {
    events
      .listen("update", (event) => applyUpdateView(event.payload))
      .then(() => events.emit("panel-update", { action: "view" }))
      .catch(() => {});
    button.addEventListener("click", async () => {
      button.disabled = true;
      $("about-update-status").textContent = "Asking the updater…";
      try {
        await events.emit("panel-update", { action: "check" });
      } catch (error) {
        $("about-update-status").textContent = `The updater could not be asked: ${error.message ?? error}`;
        button.disabled = false;
        return;
      }
      // a shell older than this panel has no one listening: say where the updater is
      clearTimeout(updateAnswerTimer);
      updateAnswerTimer = setTimeout(() => {
        $("about-update-status").textContent = "Use the Help menu above: Check for updates….";
        button.disabled = false;
      }, 4000);
    });
    return;
  }
  // a browser: no updater here
  $("about-update").dataset.state = "idle";
  $("about-update-status").textContent =
    "Updates are installed by the desktop application. In a browser, the releases page has every version.";
  $("about-update-note").textContent = "A gateway updates by its own package or tarball (docs/user/gateway-kit.md).";
  button.replaceChildren(document.createTextNode("Open the releases page"));
  button.title = "Open the project's releases page, with every version's installers and notes";
  button.addEventListener("click", () => window.open(RELEASES_URL, "_blank", "noopener"));
}

// Without a shell the version shown is the modem's own; the channel is the file's.
function applyAboutFromStatus(status) {
  if (shellEvents()?.listen) return;
  $("about-version").textContent = status.version ? `aetherd ${status.version}` : "—";
  $("about-channel").textContent = liveConfig?.update?.channel ?? "—";
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
  if (tab.dataset.panel === "diagnostics") renderRegCard();
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
  // Enter sends and the received-text note: ticked unless turned off, and remembered
  for (const [id, key] of [
    ["enter-sends", "aether.entersends"],
    ["rx-sound", "aether.rxsound"],
  ]) {
    const box = $(id);
    try {
      box.checked = localStorage.getItem(key) !== "off";
    } catch {
      box.checked = true;
    }
    box.addEventListener("change", () => {
      try {
        localStorage.setItem(key, box.checked ? "on" : "off");
      } catch {
        // storage blocked: the choice holds for this page only
      }
      unlockChime();
      if (id === "rx-sound" && box.checked) playReceivedNote(); // hear what it sounds like
    });
  }
  const chimeBox = $("chime-enabled");
  chimeBox.checked = chimeEnabled();
  chimeBox.addEventListener("change", () => {
    try {
      localStorage.setItem("aether.chime", chimeBox.checked ? "on" : "off");
    } catch {
      // storage blocked: the choice holds for this page only
    }
    unlockChime();
    if (chimeBox.checked) chime("up"); // hear what it sounds like
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
      `Run a test session with ${remote}? It sends a probe, a message, a short burst at every mode and a file — about five minutes of transmitting, ten at most, all recorded. Stop test ends it at any time.`,
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
  $("btn-clear-received").addEventListener("click", clearReceived);
  $("btn-copy-sent").addEventListener("click", copySent);
  $("btn-clear-sent").addEventListener("click", clearSent);
  $("outgoing").addEventListener("input", fitComposer);
  // a click anywhere in the Send box that is not a selection of what was sent puts the
  // cursor on the typing line
  $("composer").addEventListener("click", (event) => {
    if (event.target === $("outgoing")) return;
    const selection = window.getSelection();
    if (selection && !selection.isCollapsed && $("sent-log").contains(selection.anchorNode)) return;
    $("outgoing").focus();
  });
  loadSent();
  fitComposer();
  $("btn-reset-counters").addEventListener("click", async () => {
    await act(() => call("counters.reset"), "counters reset");
  });
  $("chart-tab-snr").addEventListener("click", () => selectStatusChart("snr", true));
  $("chart-tab-speed").addEventListener("click", () => selectStatusChart("speed", true));
  selectStatusChart(storedStatusChart());
  // notes typed before an automatic recording starts go with it
  $("record-notes").addEventListener("change", () => {
    const notes = $("record-notes").value.trim();
    call("record.notes", { notes: notes || null }).catch(() => {});
  });
  $("btn-send").addEventListener("click", sendOutgoing);
  $("outgoing").addEventListener("keydown", (event) => {
    if (event.key !== "Enter" || event.shiftKey || event.isComposing) return;
    if (!$("enter-sends").checked) return;
    event.preventDefault(); // Enter sends; Shift+Enter still starts a new line
    sendOutgoing();
  });
  $("btn-heard-clear").addEventListener("click", clearHeard);
  $("btn-sessions-clear").addEventListener("click", clearSessions);
  $("btn-sessions-all").addEventListener("click", () => {
    sessionsFilter = null;
    renderSessions();
  });
  for (const button of document.querySelectorAll("#heard-table button.sort")) {
    button.addEventListener("click", () => sortHeard(button.dataset.sort));
  }
  $("radio-bandwidth").addEventListener("change", () => {
    // the narrow ladder has fifteen rungs and the wide one twenty (ADR-0014, ADR-0015): a
    // fastest mode past the ladder would be refused on save
    const modes = $("radio-bandwidth").value === "500" ? 15 : 20;
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
  // the stations' "3 min ago", the dial and the rules' verdict on it move on their own: a dial
  // turned on the radio shows its LEGAL / WARNING / TX BLOCKED within five seconds
  setInterval(() => {
    if (panelShown("stations")) renderHeard();
    if (socket && socket.readyState === WebSocket.OPEN) refreshStatus();
  }, 5000);
  $("btn-diagnostics").addEventListener("click", copyDiagnostics);
  $("btn-contribute").addEventListener("click", contributeTestSession);
  $("contribute-link").addEventListener("click", openContributeLink);
  $("wz-profile").addEventListener("change", applyProfile);
  // a field set by hand is the operator's own setup: the list follows it
  for (const id of ["dev-in", "dev-out", "dev-ptt", "ptt-line", "ptt-protocol", "ptt-baud", "ptt-civ", "ptt-gpio", "ptt-address"]) {
    $(id).addEventListener("change", interfaceEdited);
  }
  for (const id of ["radio-busy-db", "radio-max-key", "radio-cwid", "radio-cwid-interval", "radio-cwid-wpm"]) {
    $(id).addEventListener("input", checkModemSettings);
  }
  for (const id of ["host-port", "update-channel", "host-enabled", "kiss-enabled", "kiss-host", "kiss-port"]) {
    $(id).addEventListener("input", checkAppSettings);
  }
  $("btn-kiss-disconnect").addEventListener("click", () => kissDisconnect(null));
  wireSignalCard();
  wireRules();
  wireKeying();
  wireLog();
  wireAbout();
  loadHistory();
  renderFrames(); // what was kept shows before the first new frame does
  drawStatusChart(); // the axes are there from the start, frames or none
  $("wz-call").addEventListener("input", () => {
    const value = $("wz-call").value.trim().toUpperCase();
    const plausible = /^[A-Z0-9\/-]{1,9}$/.test(value);
    checkRulesStep();
    $("wz-call-note").textContent = plausible
      ? `Will go on the air as ${value}.`
      : "Letters, digits, - and /; up to nine characters.";
    writeConfig();
  });
  $("wz-ptt").addEventListener("click", () => transmitTest("ptt.test", 1.0, "Keyed for 1 s"));
  $("wz-tune").addEventListener("click", toggleTune);
  $("wz-drive").addEventListener("click", toggleDrive);
  $("tx-level").addEventListener("input", () => showTxLevel(txLevel()));
  $("tx-level").addEventListener("change", saveTxLevel);
  $("wz-save").addEventListener("click", wizardSave);
  $("profile-select").addEventListener("change", () => {
    const id = $("profile-select").value;
    if (!id || id === profiles.active) return;
    withProfileSaved("switching", () => loadProfile(id));
  });
  $("profile-save").addEventListener("click", saveProfile);
  $("profile-save-as").addEventListener("click", () => openProfileForm("save-as"));
  $("profile-new").addEventListener("click", () => withProfileSaved("starting a new one", () => openProfileForm("new")));
  $("profile-rename").addEventListener("click", () => openProfileForm("rename"));
  $("profile-duplicate").addEventListener("click", () => openProfileForm("duplicate"));
  $("profile-delete").addEventListener("click", () => openProfileForm("delete"));
  $("profile-export").addEventListener("click", exportProfile);
  $("profile-import").addEventListener("click", () => {
    $("profile-more").open = false;
    $("profile-file").value = "";
    $("profile-file").click();
  });
  $("profile-file").addEventListener("change", () => {
    const file = $("profile-file").files?.[0];
    if (file) importProfileFile(file);
  });
  $("profile-name-ok").addEventListener("click", profileFormOk);
  $("profile-name-cancel").addEventListener("click", closeProfileForm);
  $("profile-name").addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      profileFormOk();
    } else if (event.key === "Escape") {
      closeProfileForm();
    }
  });
  $("profile-prompt-save").addEventListener("click", () => answerProfilePrompt("save"));
  $("profile-prompt-discard").addEventListener("click", () => answerProfilePrompt("discard"));
  $("profile-prompt-cancel").addEventListener("click", () => answerProfilePrompt("cancel"));
  // the More menu closes when the pointer goes elsewhere
  document.addEventListener("click", (event) => {
    const menu = $("profile-more");
    if (menu.open && !menu.contains(event.target)) menu.open = false;
  });
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
    try {
      await navigator.clipboard.writeText(url);
      line.textContent = "A link to the pre-filled report is on the clipboard and beside the button: open it, attach the .json, and send. Thank you.";
    } catch {
      line.textContent = "The pre-filled report is beside the button: open it, attach the .json, and send. Thank you.";
    }
  } catch (error) {
    line.textContent = `Could not prepare the report: ${error.message ?? error}`;
  }
}

// The report opens in the browser. Under the desktop shell a new-window link is the
// opener's to open, and a shell that may not open it (before beta.62) did nothing at all
// when it was clicked: this asks it, and says where the link is when it cannot.
async function openContributeLink(event) {
  const opener = window.__TAURI__?.opener;
  if (!opener?.openUrl) return; // a browser opens it itself
  event.preventDefault();
  const url = $("contribute-link").href;
  try {
    await opener.openUrl(url);
  } catch {
    try {
      await navigator.clipboard.writeText(url);
      $("contribute-note").textContent =
        "This version of the application cannot open the browser for you — the link is on the clipboard: paste it into your browser, attach the .json, and send. Thank you.";
    } catch {
      $("contribute-note").textContent = `Open this in your browser: ${url}`;
    }
  }
}

async function act(operation, description) {
  try {
    await operation();
    log(description);
    refreshStatus();
    return true;
  } catch (error) {
    const refusedByRules = error.code === "regulatory";
    log(error.message, refusedByRules ? "warn" : true, refusedByRules ? "rules" : "panel");
    if (refusedByRules) refreshStatus();
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
      log(text, "info", "bundle");
      note.textContent = "The clipboard is not available here; the bundle is in the log below.";
    }
  } catch (error) {
    note.textContent = error.message;
  }
}

let recordingPath = null;

// The modem runs even when its keying interface or sound card would not open, so that
// Setup — the screen naming the very port or card at fault — can be reached. It says so
// here, on every tab, because a station that is quietly deaf or mute is worse than one
// that refused to start.
function applyFaults(status) {
  const banner = $("fault-banner");
  const faults = [];
  if (status.ptt_fault) {
    faults.push(`Keying is unavailable — ${status.ptt_fault}. The modem is receiving only and will not transmit: choose the radio interface in Setup, step 2.`);
  }
  if (status.audio_fault) {
    faults.push(`The sound card is unavailable — ${status.audio_fault}. The modem can neither hear nor transmit: choose the modem devices in Setup, step 2.`);
  }
  if (status.config_note) {
    faults.push(`Settings from a backup — ${status.config_note}`);
  }
  banner.textContent = faults.join(" ");
  banner.hidden = faults.length === 0;
}

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

// Under the desktop shell the opener shows the folder: it may open the folder the shell
// itself puts recordings in (its capability names that one, and only as a folder, so no
// file in it can be launched from here), and it may always show a file in its folder — the
// newest recording, or a folder of the operator's own choosing within its parent. Only a
// plain browser, which can do neither, gets the path on the clipboard, and says so.
async function openRecordingsFolder() {
  if (!recordingsDir) return;
  const note = $("record-note");
  const opener = window.__TAURI__?.opener;
  if (opener?.openPath) {
    try {
      await opener.openPath(recordingsDir);
      note.textContent = "";
      return;
    } catch {
      // a folder of the operator's own choosing: shown in its parent below
    }
  }
  if (opener?.revealItemInDir) {
    for (const item of [await newestRecording(), recordingsDir]) {
      if (!item) continue;
      try {
        await opener.revealItemInDir(item);
        note.textContent = "";
        return;
      } catch {
        // gone since, or not there: try the next
      }
    }
  }
  await copyRecordingsPath(true);
}

/// The newest recording's audio, from the session history: the file to show the folder at.
async function newestRecording() {
  try {
    const { sessions = [] } = await call("sessions.list");
    const name = sessions.find((s) => s.recording)?.recording;
    if (!name) return null;
    const separator = recordingsDir.includes("\\") ? "\\" : "/";
    return `${recordingsDir.replace(/[\\/]+$/, "")}${separator}${name}.wav`;
  } catch {
    return null;
  }
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

// A message ends its line, so the other end shows each message on a line of its own —
// with the time it arrived — instead of running them together: the link is a byte stream
// and carries no message boundaries of its own.
async function sendOutgoing() {
  const box = $("outgoing");
  const text = box.value;
  if (!text.trim()) return;
  if ($("btn-send").disabled) {
    $("send-note").textContent = "Connect to a station first — the text stays here.";
    return;
  }
  const message = /[\r\n]$/.test(text) ? text : `${text}\n`;
  const bytes = new TextEncoder().encode(message).length;
  // the modem follows the message under this reference and says, in a `sent` event, when
  // the other station has all of it
  const ref = `p${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
  // The line is taken at once and the message shown on its way: what is typed while the
  // modem answers starts the next line. Cleared only after the answer, a quick typist's
  // next words were sent glued to the last message.
  box.value = "";
  fitComposer();
  const entry = {
    ref,
    at: Date.now(),
    to: lastRemote ?? "",
    text: message.replace(/\r?\n$/, ""),
    bytes,
    state: "pending",
  };
  addSent(entry);
  const ok = await act(() => call("send", { data: toBase64(message), ref }), `queued ${bytes} bytes`);
  if (!ok) {
    // not taken: the text goes back on the line, ahead of anything typed since, and the
    // message comes off the list
    sentEntries = sentEntries.filter((e) => e !== entry);
    saveSent();
    renderSent();
    box.value = text + box.value;
    fitComposer();
  }
}

// ── the Send box ─────────────────────────────────────────────────────
// What this station sends stays in the Send box, above the line being typed: each message
// with the time it went, in the accent colour, and a mark for what became of it — … on its
// way, ✓ once the other station has all of it (the modem's `sent` event, from the link's
// acknowledgements in order), ✗ if the session ended first. It used to vanish from the box
// the moment it was queued; the author asked for it to stay there, not in a box of its own
// (2026-09-25). Kept in this browser (the last two hundred): the daemon's log records that
// text was sent, delivered or not, and how much, never the text itself, and Clear takes
// nothing back from the session or its recording.

const SENT_KEY = "aether.sent";
/** The station of the session the text went to: the one the status last named. */
let lastRemote = null;
const SENT_KEEP = 200;
let sentEntries = [];

function loadSent() {
  try {
    const kept = JSON.parse(localStorage.getItem(SENT_KEY) ?? "[]");
    sentEntries = Array.isArray(kept) ? kept.slice(-SENT_KEEP) : [];
  } catch {
    sentEntries = [];
  }
  renderSent();
}

function saveSent() {
  try {
    localStorage.setItem(SENT_KEY, JSON.stringify(sentEntries.slice(-SENT_KEEP)));
  } catch {
    // private window or blocked storage: the list lives until the page does
  }
}

function addSent(entry) {
  sentEntries.push(entry);
  if (sentEntries.length > SENT_KEEP) sentEntries = sentEntries.slice(-SENT_KEEP);
  saveSent();
  renderSent();
}

const SENT_MARK = { pending: "…", delivered: "✓", failed: "✗", unknown: "?" };

function sentVerdict(entry) {
  const to = entry.to || "the other station";
  switch (entry.state) {
    case "pending":
      return `on its way — ${to} does not have all of it yet`;
    case "delivered":
      return `delivered: ${to} has all of it`;
    case "failed":
      return `not delivered: ${entry.reason || "the session ended first"}`;
    default:
      return "no word from the modem on this one: it restarted, or this window was closed when it was settled";
  }
}

function renderSent() {
  const pane = $("sent-log");
  pane.replaceChildren(
    ...sentEntries.map((entry) => {
      const line = document.createElement("div");
      line.className = "tx-line";
      // entries kept by beta.58 had no state: they were queued, and nothing more is known
      const state = entry.state ?? "unknown";
      line.dataset.state = state;
      const when = document.createElement("span");
      when.className = "when";
      when.textContent = `${new Date(entry.at).toLocaleTimeString()}  `;
      const body = document.createElement("span");
      body.className = "tx-text";
      body.textContent = entry.text;
      const mark = document.createElement("span");
      mark.className = "tx-mark";
      mark.textContent = SENT_MARK[state] ?? "";
      mark.setAttribute("aria-label", sentVerdict({ ...entry, state }));
      const to = entry.to ? ` to ${entry.to}` : "";
      line.title = `Sent${to} at ${new Date(entry.at).toLocaleString()}, ${entry.bytes} bytes — ${sentVerdict({ ...entry, state })}`;
      line.append(when, body, mark);
      return line;
    }),
  );
  pane.scrollTop = pane.scrollHeight;
}

/** The modem settled a message: the other station has all of it, or the session ended. */
function onSent(data) {
  const entry = sentEntries.find((e) => e.ref && e.ref === data.ref);
  if (!entry) return;
  entry.state = data.delivered ? "delivered" : "failed";
  entry.reason = data.reason ?? null;
  saveSent();
  renderSent();
}

/**
 * A `sent` event missed — the window was reloaded, or the modem restarted — is looked up in
 * the status: a message the modem still follows stays on its way, one it settled lately
 * takes that verdict, and one it has no word of is marked unknown. A message queued in the
 * last few seconds is left alone: a status asked for before it was sent would not know it.
 */
function reconcileSent(status) {
  const sent = status.sent;
  if (!sent) return;
  const pending = new Set(sent.pending ?? []);
  const recent = new Map((sent.recent ?? []).map((d) => [d.ref, d]));
  let changed = false;
  for (const entry of sentEntries) {
    if (entry.state !== "pending" || !entry.ref || Date.now() - entry.at < 5000) continue;
    const settled = recent.get(entry.ref);
    if (settled) {
      entry.state = settled.delivered ? "delivered" : "failed";
      entry.reason = settled.reason ?? null;
      changed = true;
    } else if (!pending.has(entry.ref)) {
      entry.state = "unknown";
      changed = true;
    }
  }
  if (changed) {
    saveSent();
    renderSent();
  }
}

/** The typing line grows with what is typed, a few lines at most, and says when it is empty. */
function fitComposer() {
  const box = $("outgoing");
  box.style.height = "auto";
  box.style.height = `${box.scrollHeight}px`;
  $("composer-input").dataset.empty = String(box.value === "");
}

/** Put the cursor on the typing line when a session comes up and nothing else has it. */
function focusComposer() {
  const onSession = $("tab-session").getAttribute("aria-selected") === "true";
  const free = !document.activeElement || document.activeElement === document.body;
  if (onSession && free) $("outgoing").focus();
}

async function copySent() {
  const text = sentEntries.map((entry) => entry.text).join("\n");
  try {
    await navigator.clipboard.writeText(text);
    $("send-note").textContent = "Sent text copied.";
  } catch {
    $("send-note").textContent = "Could not copy — select the text and press Ctrl+C.";
  }
}

function clearSent() {
  sentEntries = [];
  saveSent();
  renderSent();
}

// the messages, a line each, without the times in the gutter
async function copyReceived() {
  const text = [...$("incoming").children]
    .map((line) => line.querySelector(".rx-text")?.textContent ?? line.textContent)
    .join("\n");
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
  // the same words on hover: a lamp that lights for a reason should say the reason
  if (lamp.title !== label) lamp.title = label;
}

// The received pane shows reconstructed user text only. Payload bytes are decoded as
// UTF-8 with a strict, streaming decoder: a partial character at a frame boundary is
// held for the next frame, but a byte sequence that is not text — a binary or corrupted
// stream — throws rather than rendering as replacement-character soup, and is reported
// as a count instead. This is what keeps garbage out of the message window.
let rxDecoder = null;
let rxDropped = 0;
// The line text is being added to, or null once the last one has ended. A line ends at a
// line break (CR LF, CR or LF; a CR LF split between two frames is one break), and every
// message this panel sends ends with one, so each message starts a line with its own time.
let rxLine = null;
let rxCarriageReturn = false;
let rxNoteAt = 0;

function resetReceived() {
  rxDecoder = new TextDecoder("utf-8", { fatal: true });
  rxDropped = 0;
  rxLine = null;
  rxCarriageReturn = false;
}

// A line where a session begins, so one conversation is not read as the end of the last.
function markSession(remote) {
  const pane = $("incoming");
  const mark = document.createElement("div");
  mark.className = "rx-session";
  mark.textContent = `── ${remote} · ${new Date().toLocaleTimeString()} ──`;
  pane.append(mark);
  rxLine = null;
  pane.scrollTop = pane.scrollHeight;
}

// Empty the window. The session and the decoder go on: a message half arrived still
// finishes, on a line of its own.
function clearReceived() {
  $("incoming").replaceChildren();
  rxLine = null;
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
  let body = text;
  if (rxCarriageReturn && body.startsWith("\n")) body = body.slice(1);
  rxCarriageReturn = body.endsWith("\r");
  const parts = body.replace(/\r\n?/g, "\n").split("\n");
  parts.forEach((part, index) => {
    if (index > 0) rxLine = null; // a line break ended the line before
    if (part === "" && index === parts.length - 1) return; // the next text starts a line
    if (!rxLine) rxLine = newReceivedLine(pane, part !== "");
    rxLine.append(part);
  });
  if (atBottom) pane.scrollTop = pane.scrollHeight;
}

// A new line of received text: the time it began to arrive in the gutter (a blank line has
// none), and the note that says text has come, once for a flurry of lines.
function newReceivedLine(pane, stamped) {
  const line = document.createElement("div");
  line.className = "rx-line";
  if (stamped) {
    const when = document.createElement("span");
    when.className = "when";
    when.textContent = `${new Date().toLocaleTimeString()}  `;
    line.append(when);
  }
  const body = document.createElement("span");
  body.className = "rx-text";
  line.append(body);
  pane.append(line);
  if (stamped && $("rx-sound").checked && Date.now() - rxNoteAt > 1500) {
    rxNoteAt = Date.now();
    playReceivedNote();
  }
  return body;
}

// one short high note: not either of the session chime's two-note figures
function playReceivedNote() {
  playNotes([[1175, 0]], 0.22, 0.18);
}

function noteDropped() {
  const pane = $("incoming");
  let tag = pane.lastElementChild;
  if (!tag || !tag.classList.contains("rx-drop")) {
    tag = document.createElement("div");
    tag.className = "rx-drop";
    pane.appendChild(tag);
  }
  tag.textContent = `[${rxDropped} bytes of non-text data were not shown]`;
  rxLine = null;
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
