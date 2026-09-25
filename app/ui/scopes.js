// The signal-analysis card: the last frame's constellation, the received spectrum and its
// waterfall, with the waterfall's controls.
//
// The panel shows the card on its Diagnostics tab, and `signal.html` shows the same card in a
// window of its own when it is undocked, so it can stay in view whichever tab the panel is
// on. Both are this module — the markup, the drawing and the controls — so the two cannot
// drift apart. Everything it draws is polled from the control API (`spectrum` eight times a
// second, `constellation` once per frame), and only while somebody is looking: the host
// starts and stops it.

export const SPECTRUM_TOP_HZ = 4000;

// The waterfall's colours: black, the accent, white is the panel's own; the others are the
// ones operators know from other waterfalls.
export const PALETTES = {
  aether: null,
  blue: [[0, 0, 0], [0, 0, 110], [0, 70, 255], [0, 220, 255], [255, 255, 255]],
  turbo: [[48, 18, 59], [70, 107, 227], [26, 228, 182], [164, 252, 60], [251, 190, 26], [227, 72, 6], [122, 4, 3]],
  viridis: [[68, 1, 84], [59, 82, 139], [33, 145, 140], [94, 201, 98], [253, 231, 37]],
  grey: [[0, 0, 0], [255, 255, 255]],
};

const $ = (id) => document.getElementById(id);

/// The design tokens a canvas needs, read from the style sheet so a chart follows the theme.
export function tokens() {
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
    warn: read("--status-warning", "#fbbf24"),
    busy: read("--status-busy", "#fb923c"),
    plot: read("--plot-bg", "#000"),
    numerals: read("--numerals", "monospace"),
  };
}

/// A canvas sized to its box at the device's pixel ratio; returns the drawing context and
/// the size in CSS pixels. `height` is the fallback when the style sheet gives none.
export function surface(id, fallbackWidth, height) {
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

// The card's body. One copy of the markup for the docked card and the undocked window; every
// control and reading carries its tooltip (CONTRIBUTING §8).
const BODY = `
  <figure class="plot plot-square">
    <figcaption>Constellation <span class="plot-meta numerals" id="const-caption" title="The last frame: its kind, mode, SNR and whether it decoded"></span></figcaption>
    <canvas id="chart-constellation" title="Equalised symbols of the last frame: tight clusters mean a clean decode" width="240" height="240" role="img" aria-label="Equalised constellation of the last frame"></canvas>
  </figure>
  <div class="plot-stack">
    <figure class="plot">
      <figcaption>Spectrum, 0–4 kHz <span class="plot-meta numerals" id="spectrum-caption" title="The strongest bin of the received audio, or what is being sent"></span></figcaption>
      <canvas id="chart-spectrum" class="plot-spectrum" title="Spectrum of the received audio, 0 to 4 kHz; the modem's passband is shaded" width="600" height="110" role="img" aria-label="Spectrum of the received audio"></canvas>
    </figure>
    <figure class="plot plot-grow">
      <figcaption>Waterfall</figcaption>
      <canvas id="chart-waterfall" class="plot-waterfall" title="The received audio over time, newest at the top" width="600" height="140" role="img" aria-label="Waterfall of the received audio, newest at the top"></canvas>
    </figure>
  </div>
  <div class="scope-toolbar" role="group" aria-label="Waterfall controls" title="The waterfall's controls: its floor, its range, its speed and its colours">
    <label title="The level drawn as black, in dBFS; auto follows the noise">Floor <input id="wf-floor" title="The level drawn as black, in dBFS; auto follows the noise" type="range" min="-120" max="-40" step="1" value="-90" aria-label="Waterfall floor in dBFS" /> <span class="numerals scope-value" id="wf-floor-value" title="The floor in use">auto</span></label>
    <label class="check" title="Let the floor follow the noise"><input id="wf-auto" title="Let the floor follow the noise" type="checkbox" checked /> auto</label>
    <label title="The range above the floor drawn in full colour, in dB">Gain <input id="wf-gain" title="The range above the floor drawn in full colour, in dB" type="range" min="15" max="90" step="1" value="45" aria-label="Waterfall range above the floor in dB" /> <span class="numerals scope-value" id="wf-gain-value" title="The range in use">45 dB</span></label>
    <label title="How many lines the waterfall draws per second">Speed <select id="wf-speed" class="compact" title="How many lines the waterfall draws per second" aria-label="Waterfall lines per second"><option value="8">8/s</option><option value="4">4/s</option><option value="2">2/s</option><option value="1">1/s</option></select></label>
    <label title="The waterfall's colours">Palette <select id="wf-palette" class="compact" title="The waterfall's colours" aria-label="Waterfall palette"><option value="aether">Aether</option><option value="blue">Blue</option><option value="turbo">Turbo</option><option value="viridis">Viridis</option><option value="grey">Grey</option></select></label>
  </div>`;

/// Put the card's body into `container` (an element with nothing in it yet).
export function mountSignalBody(container) {
  container.innerHTML = BODY;
}

/// The card, run for a host. The host says how to reach the modem and what it knows:
///
/// * `call(method, params)` — a control-API request, as a promise;
/// * `connected()` — whether a request can be made now;
/// * `modeName(index)` — a rung's name, for the constellation's caption, or null;
/// * `canPersist()` — whether the running configuration has a `[panel]` section, so the
///   waterfall's settings can be written to the station's profile (live keys);
/// * `onActivity(live)` — optional: told whether the scopes are drawing, for a live mark.
export function createScopes(host) {
  // the waterfall's controls, as any waterfall has them: the floor (auto follows the quietest
  // fifth of the spectrum), the range above it, lines per second, the palette
  const waterfall = { auto: true, floor: -90, gain: 45, speed: 8, palette: "aether" };
  let palette = null;
  let waterfallFloor = -90;
  let spectrumTimer = null;
  let spectrumBusy = false;
  let spectrumPolls = 0;
  let constellationBusy = false;
  let pushTimer = null;

  function load() {
    try {
      const kept = JSON.parse(localStorage.getItem("aether.waterfall") ?? "{}");
      for (const key of Object.keys(waterfall)) if (key in kept) waterfall[key] = kept[key];
    } catch {
      // nothing kept, or nothing readable: the defaults
    }
    palette = null;
  }

  function save() {
    try {
      localStorage.setItem("aether.waterfall", JSON.stringify(waterfall));
    } catch {
      // a browser that keeps nothing keeps nothing
    }
  }

  function show() {
    if (!$("wf-auto")) return;
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

  // The waterfall's controls are the station's, not the window's: they are written to the
  // modem's `[panel.waterfall]` (live keys) so they travel in a profile, and kept in the
  // browser as well so a panel that cannot reach the modem still draws as it was left.
  function push() {
    if (!host.canPersist()) return;
    clearTimeout(pushTimer);
    pushTimer = setTimeout(() => {
      pushTimer = null;
      host
        .call("config.set", {
          "panel.waterfall.auto": waterfall.auto,
          "panel.waterfall.floor_db": waterfall.floor,
          "panel.waterfall.gain_db": waterfall.gain,
          "panel.waterfall.speed": waterfall.speed,
          "panel.waterfall.palette": waterfall.palette,
        })
        .catch(() => {});
    }, 400);
  }

  function changed(redraw = true) {
    save();
    push();
    if (redraw) show();
  }

  function wire() {
    load();
    $("wf-auto").addEventListener("change", () => {
      waterfall.auto = $("wf-auto").checked;
      if (!waterfall.auto) waterfall.floor = Math.round(waterfallFloor);
      changed();
    });
    $("wf-floor").addEventListener("input", () => {
      waterfall.floor = Number($("wf-floor").value);
      changed();
    });
    $("wf-gain").addEventListener("input", () => {
      waterfall.gain = Number($("wf-gain").value);
      changed();
    });
    $("wf-speed").addEventListener("change", () => {
      waterfall.speed = Number($("wf-speed").value);
      changed(false);
    });
    $("wf-palette").addEventListener("change", () => {
      waterfall.palette = $("wf-palette").value;
      palette = null;
      changed(false);
    });
    show();
  }

  /// The configuration's `[panel.waterfall]`, when it differs from what is drawn.
  function take(section) {
    if (!section) return;
    const before = JSON.stringify(waterfall);
    if (typeof section.auto === "boolean") waterfall.auto = section.auto;
    if (Number.isFinite(section.floor_db)) waterfall.floor = section.floor_db;
    if (Number.isFinite(section.gain_db)) waterfall.gain = section.gain_db;
    if (Number.isFinite(section.speed)) waterfall.speed = section.speed;
    if (typeof section.palette === "string" && section.palette in PALETTES) waterfall.palette = section.palette;
    if (JSON.stringify(waterfall) === before) return;
    palette = null;
    save();
    show();
  }

  /// Settings another window changed, from the browser's store.
  function reload() {
    load();
    show();
  }

  function start() {
    if (spectrumTimer !== null) return;
    spectrumTimer = setInterval(pollSpectrum, 125);
    pollSpectrum();
    fetchConstellation();
    host.onActivity?.(true);
  }

  function stop() {
    if (spectrumTimer === null) return;
    clearInterval(spectrumTimer);
    spectrumTimer = null;
    host.onActivity?.(false);
  }

  async function pollSpectrum() {
    if (spectrumBusy || !host.connected()) return;
    spectrumBusy = true;
    try {
      const spectrum = await host.call("spectrum");
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

  function drawSpectrum(spectrum) {
    const canvas = $("chart-spectrum");
    const { ctx, width, height } = surface("chart-spectrum", 600, canvas.clientHeight || 110);
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
      ? "transmitting — the sound card's own output"
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

  function drawWaterfall(spectrum) {
    const bins = spectrum.bins_db ?? [];
    if (bins.length === 0) return;
    const canvas = $("chart-waterfall");
    const width = Math.max(1, canvas.clientWidth || 600);
    const height = Math.max(1, canvas.clientHeight || 140);
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

  async function fetchConstellation() {
    if (constellationBusy || !host.connected()) return;
    constellationBusy = true;
    try {
      drawConstellation(await host.call("constellation"));
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
    const name = host.modeName(frame.mode);
    caption.textContent =
      `${frame.kind} · mode ${frame.mode}${name ? ` ${name}` : ""} · ` +
      `${Number(frame.snr_db).toFixed(1)} dB · ${frame.decoded ? "decoded" : "not decoded"}`;
  }

  return {
    wire,
    take,
    reload,
    start,
    stop,
    running: () => spectrumTimer !== null,
    frame: fetchConstellation,
  };
}
