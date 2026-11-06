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
      // the constellation is drawn in Full only; Compact and Mini do not ask for it
      if (frame.event === "frame" && scopes.running() && view === "full") scopes.frame();
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

// ── views ───────────────────────────────────────────────────────────
// Full is the card; Compact the spectrum and the waterfall; Mini the waterfall alone, with a
// frequency scale of its own — for a corner of the screen while a digital program runs. The
// waterfall's controls are set in Full and hold in every view. Under the shell each view keeps
// its own window size, the operator's last for it, so a view comes back as it was left; the
// view itself is kept too, and the window reopens in it. In a browser only the layout changes.

const VIEWS = ["full", "compact", "mini"];
const VIEW_KEY = "aether.signal.view";
const SIZES_KEY = "aether.signal.sizes";
// a view's window until the operator sizes it (Full's is the one the shell opens), and the
// least it may shrink to: the small views' header — the mark, the live dot, the switch, Keep
// on top and Dock — is about 410 px with Linux's fonts and wider with Windows', and on one row
// down to 460; below it the header would wrap and take the waterfall's room
const VIEW_SIZE = {
  full: { width: 860, height: 640 },
  compact: { width: 640, height: 380 },
  mini: { width: 520, height: 230 },
};
const VIEW_MIN = {
  full: { width: 420, height: 320 },
  compact: { width: 460, height: 220 },
  mini: { width: 460, height: 150 },
};
let view = "full";
// while the window is being sized for a view, its resizes are not the operator's
let sizing = false;
let sizeTimer = null;

function keptSizes() {
  try {
    const kept = JSON.parse(localStorage.getItem(SIZES_KEY) ?? "{}");
    return kept && typeof kept === "object" ? kept : {};
  } catch {
    return {};
  }
}

/// The size to open a view at: the operator's last for it, never under its least nor over the
/// screen.
function sizeFor(which) {
  const kept = keptSizes()[which];
  const least = VIEW_MIN[which];
  const size =
    kept && Number.isFinite(kept.width) && Number.isFinite(kept.height) ? kept : VIEW_SIZE[which];
  const chrome = Math.max(0, window.outerHeight - window.innerHeight);
  return {
    width: Math.max(least.width, Math.min(size.width, window.screen?.availWidth || size.width)),
    height: Math.max(least.height, Math.min(size.height, (window.screen?.availHeight || 10000) - chrome)),
  };
}

async function sizeWindow(which) {
  const getWindow = tauri?.window?.getCurrentWindow;
  const LogicalSize = tauri?.dpi?.LogicalSize ?? tauri?.window?.LogicalSize;
  if (!getWindow || !LogicalSize) return;
  sizing = true;
  try {
    const win = getWindow();
    const least = VIEW_MIN[which];
    const size = sizeFor(which);
    await win.setMinSize(new LogicalSize(least.width, least.height));
    await win.setSize(new LogicalSize(size.width, size.height));
  } catch {
    // an older shell without the window capability, or a refusal: the layout changes alone
  } finally {
    setTimeout(() => (sizing = false), 400);
  }
}

/// The operator resized the window: that is this view's size from now on.
function keepSize() {
  if (!tauri?.window?.getCurrentWindow || sizing) return;
  clearTimeout(sizeTimer);
  sizeTimer = setTimeout(async () => {
    if (sizing) return;
    try {
      if (await tauri.window.getCurrentWindow().isMaximized()) return;
      const sizes = keptSizes();
      sizes[view] = { width: window.innerWidth, height: window.innerHeight };
      localStorage.setItem(SIZES_KEY, JSON.stringify(sizes));
    } catch {
      // nothing kept: the view opens at its first size next time
    }
  }, 600);
}

function setView(next, { resize = true } = {}) {
  view = VIEWS.includes(next) ? next : "full";
  document.body.dataset.view = view;
  for (const button of document.querySelectorAll("#signal-view [data-view]")) {
    button.setAttribute("aria-pressed", String(button.dataset.view === view));
  }
  try {
    localStorage.setItem(VIEW_KEY, view);
  } catch {
    // the view holds for this window only
  }
  if (resize) sizeWindow(view);
  // back in Full, the constellation shows the last frame, not the one before the switch
  if (view === "full" && scopes.running()) scopes.frame();
}

for (const button of document.querySelectorAll("#signal-view [data-view]")) {
  button.addEventListener("click", () => setView(button.dataset.view));
}
window.addEventListener("resize", keepSize);
let keptView = "full";
try {
  keptView = localStorage.getItem(VIEW_KEY) ?? "full";
} catch {
  // nothing kept: Full
}
// the shell opens the window at Full's first size: the kept view, at its kept size
setView(keptView, { resize: true });

announce(true);
setInterval(() => announce(true), 2000);
window.addEventListener("pagehide", () => announce(false));

setLink(false);
connect();
