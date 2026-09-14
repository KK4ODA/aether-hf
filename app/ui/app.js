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

  socket.addEventListener("open", () => {
    reconnectDelay = 500;
    setLink(true);
    log("connected to the modem");
    refreshStatus();
    loadCapabilities();
    loadDevices();
    loadConfig();
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
  $("lamp-link").classList.toggle("on", up);
  if (!up) {
    setState("offline", "Not connected", `No modem at ${endpoint()}. Retrying.`);
    $("footer-version").textContent = "not connected";
    $("lamp-ptt").classList.remove("on");
    $("lamp-busy").classList.remove("on");
  }
  for (const id of ["btn-connect", "btn-disconnect", "btn-abort", "btn-beacon", "btn-send"]) {
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
      $("lamp-ptt").classList.toggle("on", data.on === true);
      break;
    case "state":
      log(`${data.name}: ${data.detail}`);
      refreshStatus();
      break;
    case "data":
      appendReceived(fromBase64(data.data ?? ""));
      break;
    case "log":
      log(`${data.name}: ${data.detail}`, data.name === "error");
      break;
    default:
      log(`${frame.event}: ${JSON.stringify(data)}`);
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

async function refreshStatus() {
  let status;
  try {
    status = await call("status");
  } catch {
    return;
  }
  const [name, detail] = STATE_TEXT[status.state] ?? ["Unknown", ""];
  const who = status.remote ? ` with ${status.remote}` : "";
  const role = { iss: " — sending", irs: " — receiving" }[status.role] ?? "";
  setState(status.state, name + who, detail + role);

  $("callsign").textContent = status.callsign || "—";
  $("footer-version").textContent = `aetherd ${status.version} · ptt: ${status.ptt}`;
  $("lamp-ptt").classList.toggle("on", status.transmitting === true);
  $("lamp-busy").classList.toggle("on", status.channel_busy === true);

  $("v-queued").textContent = String(status.queued_bytes ?? 0);
  const saving = status.compression_saving ?? 0;
  $("v-compress").textContent = status.compressing ? `${Math.round(saving * 100)}%` : "off";
  $("v-compress-sub").textContent = status.compressing
    ? "smaller on the air"
    : "not negotiated";

  applyMetrics(status.metrics ?? {});
  renderCounters(status.counters ?? {});

  $("btn-connect").disabled = status.state !== "idle";
  $("btn-beacon").disabled = status.state !== "idle";
  $("btn-disconnect").disabled = status.state === "idle";
  $("btn-abort").disabled = status.state === "idle";
  $("btn-send").disabled = status.state !== "connected";
  $("send-note").textContent =
    status.state === "connected" ? "" : "Connect to a station first.";
}

let modeTable = [];

function applyMetrics(metrics) {
  const mode = metrics.mode;
  if (mode !== undefined) {
    $("v-mode").textContent = String(mode);
    const entry = modeTable[mode];
    $("v-mode-name").textContent = entry
      ? `${entry.name} · ${Math.round(entry.net_bit_rate)} bit/s`
      : "";
  }
  if (metrics.queued_bytes !== undefined) {
    $("v-queued").textContent = String(metrics.queued_bytes);
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
    history.push({ level, floor });
    if (history.length > 240) history.shift();
    drawChart();
  }
  $("lamp-busy").classList.toggle("on", metrics.channel_busy === true);
  if (metrics.transmitting !== undefined) {
    $("lamp-ptt").classList.toggle("on", metrics.transmitting === true);
  }
}

const COUNTER_LABELS = {
  frames_sent: "Frames sent",
  frames_resent: "Of those, retransmitted",
  frames_received: "Frames received",
  frames_failed: "Frames that did not decode",
  harq_rescues: "Recovered by combining",
  bytes_delivered: "Payload bytes delivered",
  bursts: "Bursts",
  turns: "Turns taken",
  ack_timeouts: "Acknowledgements missed",
  transmissions: "Transmissions",
  deferred_for_busy: "Held back for a busy channel",
  watchdog_trips: "Key-time watchdog trips",
};

function renderCounters(counters) {
  const body = $("counters");
  body.replaceChildren();
  for (const [key, label] of Object.entries(COUNTER_LABELS)) {
    const value = counters[key];
    if (value === undefined) continue;
    const row = document.createElement("tr");
    const name = document.createElement("td");
    name.textContent = label;
    const number = document.createElement("td");
    number.className = "num";
    number.textContent = String(value);
    if (key === "watchdog_trips" && value > 0) number.style.color = "var(--hot)";
    row.append(name, number);
    body.append(row);
  }
}

// ── chart ───────────────────────────────────────────────────────────

const history = [];

function drawChart() {
  const canvas = $("chart");
  const ratio = window.devicePixelRatio || 1;
  const width = canvas.clientWidth || 900;
  const height = 180;
  if (canvas.width !== Math.round(width * ratio)) {
    canvas.width = Math.round(width * ratio);
    canvas.height = Math.round(height * ratio);
  }
  const ctx = canvas.getContext("2d");
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.clearRect(0, 0, width, height);
  if (history.length < 2) return;

  const style = getComputedStyle(document.body);
  const ink = style.getPropertyValue("--ink-faint").trim();
  const accent = style.getPropertyValue("--accent").trim();
  const edge = style.getPropertyValue("--edge").trim();

  // One scale for both traces, so the gap between them is readable as the signal margin.
  let low = Infinity;
  let high = -Infinity;
  for (const point of history) {
    low = Math.min(low, point.level, point.floor);
    high = Math.max(high, point.level, point.floor);
  }
  const pad = Math.max(3, (high - low) * 0.15);
  low -= pad;
  high += pad;

  const left = 44;
  const plotWidth = width - left - 8;
  const y = (db) => 14 + (1 - (db - low) / (high - low)) * (height - 34);
  const x = (index) => left + (index / (history.length - 1)) * plotWidth;

  ctx.font = "11px ui-monospace, monospace";
  ctx.textBaseline = "middle";
  for (let step = 0; step <= 4; step++) {
    const db = low + ((high - low) * step) / 4;
    const at = y(db);
    ctx.strokeStyle = edge;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(left, at);
    ctx.lineTo(left + plotWidth, at);
    ctx.stroke();
    ctx.fillStyle = ink;
    ctx.textAlign = "right";
    ctx.fillText(db.toFixed(0), left - 6, at);
  }

  const trace = (pick, colour, dashed) => {
    ctx.strokeStyle = colour;
    ctx.lineWidth = dashed ? 1.5 : 2;
    ctx.setLineDash(dashed ? [4, 4] : []);
    ctx.beginPath();
    history.forEach((point, index) => {
      const at = y(pick(point));
      if (index === 0) ctx.moveTo(x(index), at);
      else ctx.lineTo(x(index), at);
    });
    ctx.stroke();
    ctx.setLineDash([]);
  };

  trace((p) => p.floor, ink, true);
  trace((p) => p.level, accent, false);

  ctx.textAlign = "left";
  ctx.fillStyle = accent;
  ctx.fillText("level", left + 4, 10);
  ctx.fillStyle = ink;
  ctx.fillText("noise floor", left + 48, 10);
}

window.addEventListener("resize", drawChart);

// ── capabilities and devices ────────────────────────────────────────

async function loadCapabilities() {
  let caps;
  try {
    caps = await call("capabilities");
  } catch {
    return;
  }
  modeTable = caps.modes ?? [];
  const usable = new Set(caps.usable_modes ?? []);
  const body = $("modes");
  body.replaceChildren();
  for (const mode of modeTable) {
    const row = document.createElement("tr");
    row.dataset.usable = String(usable.has(mode.index));
    for (const [text, numeric] of [
      [String(mode.index), true],
      [mode.name, false],
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
    for (const name of entries) {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = name;
      select.append(option);
    }
    select.addEventListener("change", writeConfig);
  };
  const all = devices.devices ?? [];
  fill($("dev-in"), all.filter((d) => d.input).map((d) => d.name), "system default");
  fill($("dev-out"), all.filter((d) => d.output).map((d) => d.name), "system default");
  fill($("dev-ptt"), devices.serial_ports ?? [], "none (VOX or receive only)");
  writeConfig();
}

// What the daemon is actually running, as `config.get` reported it.
let liveConfig = null;
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
    $("btn-apply").disabled = true;
    writeConfig();
    return;
  }
  liveConfig = answer.config ?? {};
  liveKeys = answer.live_keys ?? [];
  configPath = answer.path ?? "";
  $("btn-apply").disabled = false;

  // show the operator what is there now, so the form is not a blank slate over live settings
  if (!$("setup-call").value) $("setup-call").value = liveConfig.callsign ?? "";
  select($("dev-in"), liveConfig.audio?.input ?? "");
  select($("dev-out"), liveConfig.audio?.output ?? "");
  select($("dev-ptt"), liveConfig.ptt?.port ?? "");
  writeConfig();
}

function select(element, value) {
  if ([...element.options].some((option) => option.value === value)) element.value = value;
}

function writeConfig() {
  $("config-out").textContent = liveConfig
    ? JSON.stringify(liveConfig, null, 2)
    : "The daemon has no configuration file to change.";
  $("footer-config").textContent = configPath;
}

/// Everything the form would change, as the dotted keys `config.set` takes.
function formChanges() {
  const port = $("dev-ptt").value;
  const changes = {
    "audio.input": $("dev-in").value || null,
    "audio.output": $("dev-out").value || null,
    "ptt.kind": port ? "serial" : "none",
  };
  const callsign = $("setup-call").value.trim().toUpperCase();
  if (callsign) changes.callsign = callsign;
  if (port) {
    changes["ptt.port"] = port;
    changes["ptt.line"] = "rts";
  }
  return changes;
}

async function applyConfig() {
  $("apply-note").textContent = "";
  let answer;
  try {
    answer = await call("config.set", formChanges());
  } catch (error) {
    $("apply-note").textContent = error.message;
    $("apply-note").style.color = "var(--hot)";
    log(error.message, true);
    return;
  }
  $("apply-note").style.color = "";
  const restart = answer.restart_required ?? [];
  $("apply-note").textContent =
    restart.length === 0
      ? `Saved to ${answer.path}. In effect now.`
      : `Saved to ${answer.path}. Restart the daemon for: ${restart.join(", ")}.`;
  log(`settings saved (${(answer.changed ?? []).join(", ")})`);
  loadConfig();
  refreshStatus();
}

// ── actions ─────────────────────────────────────────────────────────

function wire() {
  for (const tab of document.querySelectorAll(".tab")) {
    tab.addEventListener("click", () => {
      for (const other of document.querySelectorAll(".tab")) {
        const selected = other === tab;
        other.setAttribute("aria-selected", String(selected));
        $(`panel-${other.dataset.panel}`).hidden = !selected;
      }
      if (tab.dataset.panel === "status") drawChart();
    });
  }

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
  $("btn-apply").addEventListener("click", applyConfig);
  $("btn-copy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(asToml(formChanges()));
      $("copy-note").textContent = "copied";
    } catch {
      $("copy-note").textContent = "could not copy — select it and copy by hand";
    }
    setTimeout(() => ($("copy-note").textContent = ""), 3000);
  });
  $("setup-call").addEventListener("input", writeConfig);
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

function appendReceived(text) {
  const pane = $("incoming");
  pane.textContent += text;
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
  lines.push("tx_level = 0.25", "", "[ptt]");
  if (changes["ptt.port"]) {
    lines.push(
      `kind = "serial"`,
      `port = ${quote(changes["ptt.port"])}`,
      `line = ${quote(changes["ptt.line"] ?? "rts")}`,
    );
  } else {
    lines.push(`kind = "none"   # VOX, or receive only`);
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

function fromBase64(text) {
  try {
    const binary = atob(text);
    const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
    return new TextDecoder().decode(bytes);
  } catch {
    return "";
  }
}

wire();
setLink(false);
connect();
