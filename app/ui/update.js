// Aether HF — the updates window.
//
// A page of the desktop shell's own (bundled with the panel, served from the binary), so it
// works while the modem is stopped and the network is down. The shell keeps the state and
// pushes every change as an `update` event; this page asks for the state once on load and
// acts through the shell's `update_*` commands. Nothing here talks to the modem.
//
// Opened in a browser rather than the shell — the daemon serves this directory too — there
// is no shell to ask, and `?demo=<phase>` shows what each phase looks like instead.

const $ = (id) => document.getElementById(id);

const tauri = window.__TAURI__ ?? null;

async function invoke(command, args = {}) {
  if (!tauri) return null;
  return tauri.core.invoke(command, args);
}

// ── phases ──────────────────────────────────────────────────────────

const PHASES = {
  checking: {
    title: "Checking for updates…",
    text: () => "Asking the release channel.",
    buttons: ["later"],
    busy: true,
  },
  "up-to-date": {
    title: "You have the newest version",
    text: (v) => `Aether HF ${v.current} is the newest on the ${v.channel} channel.`,
    buttons: ["restore", "check", "close"],
  },
  available: {
    title: (v) => `Aether HF ${v.version} is available`,
    text: (v) =>
      `You have ${v.current}.${v.date ? ` Published ${published(v.date)}.` : ""} Installing ` +
      "stops the modem, installs over this version and starts Aether HF again; your " +
      "settings are kept, and so is the version you have now.",
    buttons: ["later", "install"],
    offered: "Available",
    notes: true,
  },
  downloading: {
    title: (v) => `Downloading Aether HF ${v.version}`,
    text: () => "The installer is signed; anything the project's key did not sign is refused.",
    buttons: [],
    offered: "Downloading",
    progress: true,
    busy: true,
  },
  installing: {
    title: (v) => `Installing Aether HF ${v.version}`,
    text: () =>
      "The modem is stopping and the installer is starting. Aether HF closes and starts " +
      "again by itself; this can take a minute.",
    buttons: [],
    offered: "Installing",
    progress: "indeterminate",
    busy: true,
  },
  "restart-required": {
    title: (v) => `Aether HF ${v.version} is installed`,
    text: () => "Start Aether HF again to run it. The modem stops and starts with it.",
    buttons: ["later", "restart"],
    offered: "Installed",
  },
  complete: {
    title: (v) => `Updated to Aether HF ${v.version}`,
    text: (v) => `This is the first start since ${v.from}. Here is what changed.`,
    buttons: ["restore", "close"],
    notes: true,
    from: true,
  },
  incomplete: {
    title: (v) => `The update to ${v.version} did not install`,
    text: (v) =>
      `You still have ${v.current}. The installer was cancelled, or failed; check for ` +
      "updates to try again, or install from the releases page.",
    buttons: ["check", "close"],
  },
  error: {
    title: "Something went wrong",
    text: (v) => v.message,
    buttons: (v) => (v.retry ? ["check", "close"] : ["close"]),
  },
};

function published(rfc3339) {
  const date = new Date(rfc3339);
  if (Number.isNaN(date.getTime())) return rfc3339;
  return date.toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" });
}

function formatBytes(n) {
  if (n < 1e6) return `${(n / 1000).toFixed(0)} kB`;
  return `${(n / 1e6).toFixed(1)} MB`;
}

function render(view) {
  const spec = PHASES[view.phase] ?? PHASES.error;
  const phase = $("phase");
  phase.dataset.phase = view.phase in PHASES ? view.phase : "error";
  $("title").textContent = typeof spec.title === "function" ? spec.title(view) : spec.title;
  $("text").textContent = spec.text(view) ?? "";
  $("current").textContent = view.current ?? "—";
  $("channel").textContent = `${view.channel ?? ""} channel`;
  document.title = `Aether HF updates — ${$("title").textContent}`;

  const offered = spec.offered ?? (spec.from ? "Replaced" : null);
  $("offered-box").hidden = !offered;
  $("arrow").hidden = !offered;
  if (offered) {
    $("offered-label").textContent = offered;
    $("offered").textContent = spec.from ? view.from : view.version;
  }
  if (spec.from) {
    // the arrow points the other way: what was replaced sits on the right
    $("offered-label").textContent = "Replaced";
  }

  const progress = $("progress");
  progress.hidden = !spec.progress;
  if (spec.progress === "indeterminate") {
    progress.dataset.indeterminate = "true";
    progress.removeAttribute("aria-valuenow");
    $("progress-text").textContent = "";
  } else if (spec.progress) {
    progress.dataset.indeterminate = view.total ? "false" : "true";
    if (view.total) {
      const percent = Math.min(100, Math.round((100 * view.downloaded) / view.total));
      $("progress-fill").style.width = `${percent}%`;
      progress.setAttribute("aria-valuenow", String(percent));
      $("progress-text").textContent =
        `${formatBytes(view.downloaded)} of ${formatBytes(view.total)} · ${percent}%`;
    } else {
      $("progress-text").textContent = formatBytes(view.downloaded ?? 0);
    }
  }

  const notes = $("notes");
  notes.hidden = !spec.notes || !(view.notes ?? "").trim();
  if (!notes.hidden) renderMarkdown($("notes-body"), view.notes);

  const fineprint = $("fineprint");
  if (view.phase === "available") {
    fineprint.hidden = false;
    fineprint.textContent =
      "Every installer is kept, so Help › Restore the previous version can always go " +
      "back without a network.";
  } else if (view.phase === "installing") {
    fineprint.hidden = false;
    fineprint.textContent =
      "If nothing happens after a minute, the installer may be waiting behind another " +
      "window; if Aether HF does not come back, the releases page has every version.";
  } else {
    fineprint.hidden = true;
  }

  const wanted = new Set(typeof spec.buttons === "function" ? spec.buttons(view) : spec.buttons);
  for (const name of ["restore", "check", "later", "install", "restart", "close"]) {
    $(`btn-${name}`).hidden = !wanted.has(name);
  }
  if (wanted.has("restore") && !view.can_restore) $("btn-restore").hidden = true;
  $("btn-releases").disabled = spec.busy === true && view.phase !== "checking";
}

// ── the notes: Markdown as git-cliff writes it, rendered as DOM ─────

function renderMarkdown(into, text) {
  into.replaceChildren();
  let list = null;
  let paragraph = [];
  const flush = () => {
    if (paragraph.length) {
      const p = document.createElement("p");
      inline(p, paragraph.join(" "));
      into.append(p);
      paragraph = [];
    }
  };
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trimEnd();
    const heading = /^(#{1,4})\s+(.*)$/.exec(line);
    const bullet = /^\s*[-*]\s+(.*)$/.exec(line);
    if (heading) {
      flush();
      list = null;
      const h = document.createElement(`h${Math.min(4, heading[1].length)}`);
      inline(h, heading[2]);
      into.append(h);
    } else if (bullet) {
      flush();
      if (!list) {
        list = document.createElement("ul");
        into.append(list);
      }
      const li = document.createElement("li");
      inline(li, bullet[1]);
      list.append(li);
    } else if (line.trim() === "") {
      flush();
      list = null;
    } else {
      list = null;
      paragraph.push(line.trim());
    }
  }
  flush();
}

// bold, code and links; a link opens in the browser through the shell, and only the
// project's own pages, so a note cannot send anybody anywhere else
function inline(into, text) {
  const pattern = /(\*\*[^*]+\*\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g;
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > last) into.append(document.createTextNode(text.slice(last, match.index)));
    const token = match[0];
    if (token.startsWith("**")) {
      const strong = document.createElement("strong");
      strong.textContent = token.slice(2, -2);
      into.append(strong);
    } else if (token.startsWith("`")) {
      const code = document.createElement("code");
      code.textContent = token.slice(1, -1);
      into.append(code);
    } else {
      const parts = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(token);
      const a = document.createElement("a");
      a.textContent = parts[1];
      a.href = parts[2];
      if (/^[0-9a-f]{7,}$/.test(parts[1])) a.className = "hash";
      a.addEventListener("click", (event) => {
        event.preventDefault();
        if (tauri) invoke("update_open", { url: parts[2] }).catch(() => {});
        else window.open(parts[2], "_blank", "noopener");
      });
      into.append(a);
    }
    last = match.index + token.length;
  }
  if (last < text.length) into.append(document.createTextNode(text.slice(last)));
}

// ── wiring ──────────────────────────────────────────────────────────

const DEMO_NOTES = [
  "## [0.2.0-beta.14] - 2026-09-15",
  "",
  "### Features",
  "",
  "- **app:** a dashboard with SNR, tuning, throughput and the session ([abc1234](https://github.com/KK4ODA/aether-hf/commit/abc1234))",
  "- **app:** the stations heard, kept beside the configuration",
  "- **app:** the updates window",
  "",
  "### Bug Fixes",
  "",
  "- **link:** a message one byte short of a full frame no longer stops the modem",
].join("\n");

function demo(phase) {
  const base = { current: "0.2.0-beta.13", channel: "beta", can_restore: true, phase };
  const extra = {
    available: { version: "0.2.0-beta.14", notes: DEMO_NOTES, date: "2026-09-15T10:00:00Z" },
    downloading: { version: "0.2.0-beta.14", downloaded: 3_200_000, total: 9_100_000 },
    installing: { version: "0.2.0-beta.14" },
    "restart-required": { version: "0.2.0-beta.14" },
    complete: { current: "0.2.0-beta.14", version: "0.2.0-beta.14", from: "0.2.0-beta.13", notes: DEMO_NOTES },
    incomplete: { version: "0.2.0-beta.14" },
    error: { message: "Could not check for updates: the network is unreachable. Try again when it is back.", retry: true },
  };
  return { ...base, ...(extra[phase] ?? {}) };
}

function wire() {
  $("btn-releases").addEventListener("click", () => {
    if (tauri) invoke("update_open", {}).catch(() => {});
    else window.open("https://github.com/KK4ODA/aether-hf/releases", "_blank", "noopener");
  });
  $("btn-check").addEventListener("click", () => invoke("update_check"));
  $("btn-install").addEventListener("click", () => invoke("update_install"));
  $("btn-restart").addEventListener("click", () => invoke("update_restart"));
  $("btn-restore").addEventListener("click", () => invoke("update_restore"));
  for (const name of ["later", "close"]) {
    $(`btn-${name}`).addEventListener("click", () => {
      if (tauri) invoke("update_close");
      else window.close();
    });
  }
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && !$("btn-later").hidden) $("btn-later").click();
    if (event.key === "Escape" && !$("btn-close").hidden) $("btn-close").click();
  });
}

async function start() {
  wire();
  if (!tauri) {
    const phase = new URLSearchParams(location.search).get("demo") ?? "up-to-date";
    render(demo(phase));
    return;
  }
  await tauri.event.listen("update", (event) => render(event.payload));
  try {
    render(await invoke("update_view"));
  } catch (error) {
    render({
      phase: "error",
      current: "—",
      channel: "",
      can_restore: false,
      message: `This window lost the shell: ${error}`,
      retry: false,
    });
  }
}

start();
