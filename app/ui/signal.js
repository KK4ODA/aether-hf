// The undocked signal-analysis card (signal.html): the constellation, spectrum and waterfall
// of scopes.js in a window of their own, talking to the modem over the same control API as
// the panel. The panel knows the window is open — the desktop shell tells it, a browser's
// panel hears it on a BroadcastChannel — and draws nothing of its own meanwhile.

import { createScopes, mountSignalBody } from "./scopes.js";

const $ = (id) => document.getElementById(id);

// Served by the daemon: talk to wherever this page came from (as app.js does).
const endpoint = () => {
  if (location.protocol === "http:" || location.protocol === "https:") {
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    return `${scheme}//${location.host}/v1`;
  }
  return "ws://127.0.0.1:8515/v1";
};

const pending = new Map();
let socket = null;
let nextId = 1;
let reconnectDelay = 500;
let modeTable = [];
let hasPanelSection = false;

function call(method, params = {}) {
  return new Promise((resolve, reject) => {
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      reject(new Error("not connected to the modem"));
      return;
    }
    const id = String(nextId++);
    pending.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params }));
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error(`${method} timed out`));
    }, 10000);
  });
}

const connected = () => socket !== null && socket.readyState === WebSocket.OPEN;

mountSignalBody($("signal-body"));
const scopes = createScopes({
  call,
  connected,
  modeName: (index) => modeTable[index]?.name ?? null,
  canPersist: () => hasPanelSection,
  onActivity: (live) => {
    const mark = $("signal-live");
    mark.dataset.live = String(live);
    mark.textContent = live ? "live" : "paused";
  },
});
scopes.wire();

function setLink(up) {
  const lamp = $("lamp-link");
  lamp.classList.toggle("on", up);
  const label = up ? "Connected to the modem" : "Not connected to the modem";
  lamp.setAttribute("aria-label", label);
  lamp.title = label;
  const note = $("signal-note");
  note.textContent = up ? "" : `No modem at ${endpoint()}. Retrying.`;
  note.hidden = up;
}

// Draw while the window can be seen and the modem answers; a minimised window costs the
// modem nothing.
function wanted() {
  if (connected() && document.visibilityState === "visible") scopes.start();
  else scopes.stop();
}

async function loadContext() {
  try {
    const caps = await call("capabilities");
    modeTable = caps.modes ?? [];
  } catch {
    // captions without mode names
  }
  try {
    const answer = await call("config.get");
    hasPanelSection = Boolean(answer.config?.panel);
    scopes.take(answer.config?.panel?.waterfall);
  } catch {
    hasPanelSection = false;
  }
}

function connect() {
  socket = new WebSocket(endpoint());
  socket.addEventListener("open", async () => {
    reconnectDelay = 500;
    setLink(true);
    await loadContext();
    wanted();
  });
  socket.addEventListener("message", (message) => {
    let frame;
    try {
      frame = JSON.parse(message.data);
    } catch {
      return;
    }
    if (frame.event !== undefined) {
      if (frame.event === "frame" && scopes.running()) scopes.frame();
      // a profile loaded or the waterfall changed from the panel: follow it
      if (frame.event === "profile") loadContext();
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
    scopes.stop();
    for (const [id, waiting] of pending) {
      waiting.reject(new Error("the connection closed"));
      pending.delete(id);
    }
    setTimeout(connect, reconnectDelay);
    reconnectDelay = Math.min(reconnectDelay * 2, 10000);
  });
  socket.addEventListener("error", () => socket.close());
}

document.addEventListener("visibilitychange", wanted);
// the panel's own controls changed the waterfall: the browser's store says so
window.addEventListener("storage", (event) => {
  if (event.key === "aether.waterfall") scopes.reload();
});

// ── docking ─────────────────────────────────────────────────────────
// Under the desktop shell the shell owns this window: Dock asks it to close it, and it tells
// the panel. In a browser this page was opened by the panel, closes itself, and says it is
// here — and when it goes — on a channel the panel listens to, so a reloaded panel still
// knows the card is out.

const tauri = window.__TAURI__;
let channel = null;
try {
  channel = new BroadcastChannel("aether-signal");
} catch {
  channel = null;
}

function announce(alive) {
  try {
    channel?.postMessage({ alive });
  } catch {
    // a closed channel says nothing
  }
}

$("btn-dock").addEventListener("click", async () => {
  if (tauri?.event?.emit) {
    try {
      await tauri.event.emit("panel-signal", { action: "dock" });
      return;
    } catch {
      // an older shell: close the window ourselves below
    }
  }
  announce(false);
  window.close();
});

if (tauri?.window?.getCurrentWindow) {
  const box = $("on-top-box");
  box.hidden = false;
  let onTop = false;
  try {
    onTop = localStorage.getItem("aether.signal.ontop") === "1";
  } catch {
    // nothing kept
  }
  $("on-top").checked = onTop;
  const apply = async (on) => {
    try {
      await tauri.window.getCurrentWindow().setAlwaysOnTop(on);
    } catch {
      box.hidden = true; // the shell does not allow it: say nothing rather than pretend
    }
  };
  if (onTop) apply(true);
  $("on-top").addEventListener("change", () => {
    const on = $("on-top").checked;
    try {
      localStorage.setItem("aether.signal.ontop", on ? "1" : "0");
    } catch {
      // the choice holds for this window only
    }
    apply(on);
  });
}

announce(true);
setInterval(() => announce(true), 2000);
window.addEventListener("pagehide", () => announce(false));

setLink(false);
connect();
