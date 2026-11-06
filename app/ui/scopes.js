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

// The waterfall's colours. The panel's own is a radio's: the noise a deep navy, a weak signal
// blue, a strong one cyan to white, so the band reads as dark water with the signals standing
// out of it; the others are the ones operators know from other waterfalls.
export const PALETTES = {
  aether: [[2, 5, 18], [4, 22, 78], [14, 78, 186], [44, 160, 252], [176, 232, 255], [255, 255, 236]],
  blue: [[0, 0, 0], [0, 0, 110], [0, 70, 255], [0, 220, 255], [255, 255, 255]],
  turbo: [[48, 18, 59], [70, 107, 227], [26, 228, 182], [164, 252, 60], [251, 190, 26], [227, 72, 6], [122, 4, 3]],
  viridis: [[68, 1, 84], [59, 82, 139], [33, 145, 140], [94, 201, 98], [253, 231, 37]],
  grey: [[0, 0, 0], [255, 255, 255]],
};

const $ = (id) => document.getElementById(id);

/// A `#rrggbb` colour as its three channels; grey for anything else.
function rgb(colour) {
  const m = /^#([0-9a-f]{6})$/i.exec(colour);
  if (!m) return [64, 64, 64];
  const v = parseInt(m[1], 16);
  return [(v >> 16) & 255, (v >> 8) & 255, v & 255];
}

/// Where this station's widest transmission reaches, audio hertz, low and high: what the
/// modem measured for the bandwidth it runs (`transmit_hz`), or its nominal channel from a
/// modem that does not say.
function transmitEdges(spectrum) {
  const [lo, hi] = spectrum.transmit_hz ?? spectrum.passband_hz ?? [0, 0];
  return hi > lo ? [lo, hi] : null;
}

// The share of each new spectrum in the waterfall's running average: polled eight times a
// second, about a third of a second of memory.
const WATERFALL_AVERAGE = 0.35;
// The auto floor's place above the noise, in dB: most of the noise draws black.
const WATERFALL_FLOOR_ABOVE_NOISE_DB = 1;
// How fast the noise estimate falls to a quieter reading and rises to a louder one, per poll:
// it drops in the gaps between transmissions and climbs slowly under a long one (about twelve
// seconds), so a signal filling the channel does not lift the floor to itself.
const NOISE_FALL = 0.25;
const NOISE_RISE = 0.01;
// How far the colours' low end is bent down: the noise near the floor stays dark.
const WATERFALL_GAMMA = 1.6;

/// The design tokens a canvas needs, read from the style sheet so a chart follows the theme.
export function tokens() {
  const style = getComputedStyle(document.body);
  const read = (name, fallback) => style.getPropertyValue(name).trim() || fallback;
  return {
    ink: read("--text-3", "#888"),
    ink2: read("--text-2", "#aaa"),
    // the trace a measurement is drawn with; state colours go on top of it
    accent: read("--trace", "#c9d2da"),
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

/// "500", "1 kHz": a frequency as the spectrum's axis writes it.
function hzLabel(hz) {
  return hz % 1000 === 0 ? `${hz / 1000} kHz` : String(hz);
}

// The audio the spectrum and the waterfall show. The daemon sends 0–4 kHz, but a radio's SSB
// filter ends near 3 kHz, and the top quarter was always black while the signal, centred on
// 1500 Hz, sat left of centre (the author, 2026-11-06). 0–3 kHz is centred on it and still
// shows any SSB filter's audio whole; 0–4 kHz shows a wide data filter's skirt; Channel zooms
// on the modem's own channel — a 500 Hz signal is a sliver of 3 kHz.
const SPAN_CHOICES = ["3k", "4k", "channel"];
const SPANS = { "3k": [0, 3000], "4k": [0, SPECTRUM_TOP_HZ] };

/// Room either side of the red lines in the Channel span, in hertz: a signal beside the channel,
/// about to wander onto it, is still in view.
const CHANNEL_SPARE_HZ = 200;

/// The span to draw, in audio hertz: a fixed one, or for Channel this station's widest
/// transmission (the red lines) with CHANNEL_SPARE_HZ either side, on 50 Hz steps — 900–2100 Hz
/// at 500 Hz, 0–3 kHz at 2300.
function spanOf(choice, spectrum) {
  if (choice !== "channel") return SPANS[choice] ?? SPANS["3k"];
  const edges = transmitEdges(spectrum);
  if (!edges) return SPANS["3k"];
  const [a, b] = edges;
  return [
    Math.max(0, Math.floor((a - CHANNEL_SPARE_HZ) / 50) * 50),
    Math.min(SPECTRUM_TOP_HZ, Math.ceil((b + CHANNEL_SPARE_HZ) / 50) * 50),
  ];
}

/// "0–3 kHz", "950–2050 Hz": a span as the caption says it.
function spanLabel([lo, hi]) {
  return lo % 1000 === 0 && hi % 1000 === 0 ? `${lo / 1000}–${hi / 1000} kHz` : `${lo}–${hi} Hz`;
}

/// The frequencies a span's axis labels: every 100, 250, 500 or 1000 Hz, the finest that puts
/// no more than eight inside it, its edges left bare.
function ticksOf([lo, hi]) {
  const step = [100, 250, 500, 1000].find((s) => Math.ceil(hi / s) - Math.floor(lo / s) - 1 <= 8) ?? 1000;
  const ticks = [];
  for (let hz = Math.floor(lo / step) * step + step; hz < hi; hz += step) ticks.push(hz);
  return ticks;
}

/// The waterfall's own frequency scale: drawn under it only where the spectrum, which carries
/// the scale, is not shown (the undocked window's Mini view).
function scaleOf(span) {
  const [lo, hi] = span;
  return ticksOf(span)
    .map((hz) => `<span style="left: ${((hz - lo) / (hi - lo)) * 100}%">${hzLabel(hz)}</span>`)
    .join("");
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
      <figcaption><span>Spectrum, <span id="spectrum-span" title="The audio shown, chosen with Span under the waterfall">${spanLabel(SPANS["3k"])}</span></span> <span class="plot-meta numerals" id="spectrum-caption" title="The strongest bin of the received audio in the span shown, or what is being sent"></span></figcaption>
      <canvas id="chart-spectrum" class="plot-spectrum" title="Spectrum of the received audio across the span chosen with Span. The red lines, shaded between, are the edges of this station's widest transmission at the bandwidth it runs (the FCC's 26 dB measure): a signal between them is on your frequency" width="600" height="110" role="img" aria-label="Spectrum of the received audio"></canvas>
    </figure>
    <figure class="plot plot-grow">
      <figcaption>Waterfall</figcaption>
      <canvas id="chart-waterfall" class="plot-waterfall" title="The received audio over time, newest at the top. The red lines are the edges of this station's widest transmission at the bandwidth it runs (the FCC's 26 dB measure): a signal between them is on your frequency, so wait before calling" width="600" height="140" role="img" aria-label="Waterfall of the received audio, newest at the top"></canvas>
      <div class="wf-scale numerals" id="wf-scale" title="Audio frequency across the waterfall, over the span chosen with Span" aria-hidden="true">${scaleOf(SPANS["3k"])}</div>
    </figure>
  </div>
  <div class="scope-toolbar" role="group" aria-label="Waterfall controls" title="The waterfall's controls: the audio it and the spectrum show, its floor, its range, its speed and its colours">
    <label title="The audio the spectrum and the waterfall show: 0–3 kHz, any SSB filter whole; 0–4 kHz, a wide data filter and its skirt; Channel, this station's channel with room either side">Span <select id="wf-span" class="compact" title="The audio the spectrum and the waterfall show: 0–3 kHz, any SSB filter whole; 0–4 kHz, a wide data filter and its skirt; Channel, this station's channel with room either side" aria-label="Spectrum and waterfall span"><option value="3k">0–3 kHz</option><option value="4k">0–4 kHz</option><option value="channel">Channel</option></select></label>
    <label title="The level drawn as black, in dBFS; auto follows the noise">Floor <input id="wf-floor" title="The level drawn as black, in dBFS; auto follows the noise" type="range" min="-120" max="-40" step="1" value="-90" aria-label="Waterfall floor in dBFS" /> <span class="numerals scope-value" id="wf-floor-value" title="The floor in use">auto</span></label>
    <label class="check" title="Let the floor follow the noise"><input id="wf-auto" title="Let the floor follow the noise" type="checkbox" checked /> auto</label>
    <label title="The range above the floor drawn in full colour, in dB: smaller is more contrast">Gain <input id="wf-gain" title="The range above the floor drawn in full colour, in dB: smaller is more contrast" type="range" min="3" max="90" step="1" value="20" aria-label="Waterfall range above the floor in dB" /> <span class="numerals scope-value" id="wf-gain-value" title="The range in use">20 dB</span></label>
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
  const waterfall = { auto: true, floor: -90, gain: 20, speed: 8, palette: "aether", span: "3k" };
  let palette = null;
  let waterfallFloor = -90;
  // each bin's power averaged over the last few spectra: one 85 ms transform's bins scatter by
  // several decibels about the noise, which drew the background as grain; averaged, the noise
  // is an even field and a signal stands out of it, as on a radio's waterfall
  let averaged = null;
  let averagedBins = null;
  let noise = null;
  let spectrumTimer = null;
  let spectrumBusy = false;
  let spectrumPolls = 0;
  let constellationBusy = false;
  let pushTimer = null;
  // the span the waterfall's lines were drawn over, and the one the captions say
  let drawnSpan = null;
  let shownSpan = null;

  function load() {
    try {
      const kept = JSON.parse(localStorage.getItem("aether.waterfall") ?? "{}");
      for (const key of Object.keys(waterfall)) if (key in kept) waterfall[key] = kept[key];
    } catch {
      // nothing kept, or nothing readable: the defaults
    }
    if (!SPAN_CHOICES.includes(waterfall.span)) waterfall.span = "3k";
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
    $("wf-span").value = waterfall.span;
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
    // the span is the screen's, kept in the browser alone: a key in `[panel.waterfall]` would be
    // a new shape of the configuration, and a version going back could not read it
    $("wf-span").addEventListener("change", () => {
      waterfall.span = SPAN_CHOICES.includes($("wf-span").value) ? $("wf-span").value : "3k";
      save();
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
      const span = spanOf(waterfall.span, spectrum);
      showSpan(span);
      // a view that hides the spectrum (the undocked window's Mini) still feeds the waterfall
      if ($("chart-spectrum").clientWidth > 0) drawSpectrum(spectrum, span);
      // the average runs at every poll, whatever the waterfall's speed, and pauses while
      // this station transmits: its own signal is not the band
      const bins = spectrum.bins_db ?? [];
      if (bins.length > 0 && !spectrum.transmitting) {
        averagedBins = average(bins);
        trackNoise(averagedBins, spectrum);
      }
      // the spectrum refreshes eight times a second; the waterfall advances at its own pace
      spectrumPolls += 1;
      if (spectrumPolls % Math.max(1, Math.round(8 / waterfall.speed)) === 0) drawWaterfall(spectrum, span);
    } catch {
      // the next tick tries again; a missed frame of a scope is nothing
    } finally {
      spectrumBusy = false;
    }
  }

  /// The captions and the waterfall's scale, when the span changes.
  function showSpan(span) {
    if (shownSpan && shownSpan[0] === span[0] && shownSpan[1] === span[1]) return;
    shownSpan = span;
    $("spectrum-span").textContent = spanLabel(span);
    $("wf-scale").innerHTML = scaleOf(span);
  }

  function drawSpectrum(spectrum, [lo, hi]) {
    const canvas = $("chart-spectrum");
    const { ctx, width, height } = surface("chart-spectrum", 600, canvas.clientHeight || 110);
    const c = tokens();
    ctx.clearRect(0, 0, width, height);
    const bins = spectrum.bins_db ?? [];
    const caption = $("spectrum-caption");
    const binHz = spectrum.bin_hz;
    const low = -110;
    const high = 0;
    const x = (hz) => ((hz - lo) / (hi - lo)) * width;
    const y = (db) => (1 - (Math.max(low, Math.min(high, db)) - low) / (high - low)) * (height - 14) + 2;
    // where this station's transmission goes: shaded, its edges in the transmit colour, as
    // on the waterfall
    const edges = transmitEdges(spectrum);
    if (edges) {
      const [lo, hi] = edges;
      ctx.fillStyle = c.accent;
      ctx.globalAlpha = 0.06;
      ctx.fillRect(x(lo), 0, x(hi) - x(lo), height - 14);
      ctx.globalAlpha = 1;
    }
    // the graticule is drawn whether or not there is a trace: an instrument with nothing to
    // show still shows its scale
    ctx.strokeStyle = c.grid;
    ctx.lineWidth = 1;
    ctx.font = `10px ${c.numerals}`;
    ctx.textBaseline = "top";
    ctx.textAlign = "center";
    for (const hz of ticksOf([lo, hi])) {
      ctx.beginPath();
      ctx.moveTo(x(hz), 0);
      ctx.lineTo(x(hz), height - 14);
      ctx.stroke();
      ctx.fillStyle = c.ink;
      ctx.fillText(hzLabel(hz), x(hz), height - 12);
    }
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    // a label every 20 dB, or every 40 on a plot too short for them (the undocked Compact view)
    const labelEvery = y(-40) - y(-20) >= 14 ? 20 : 40;
    for (const db of [-20, -40, -60, -80, -100]) {
      ctx.beginPath();
      ctx.moveTo(0, y(db));
      ctx.lineTo(width, y(db));
      ctx.stroke();
      if (db % labelEvery !== 0) continue;
      ctx.fillStyle = c.ink;
      ctx.fillText(`${db}`, 4, y(db) - 6);
    }
    if (edges) {
      ctx.strokeStyle = c.tx;
      ctx.lineWidth = 1;
      for (const hz of edges) {
        const at = Math.round(x(hz)) + 0.5;
        ctx.beginPath();
        ctx.moveTo(at, 0);
        ctx.lineTo(at, height - 14);
        ctx.stroke();
      }
    }
    if (bins.length === 0) {
      caption.textContent = "waiting for audio";
      return;
    }
    // the bins in the span, and one either side so the trace reaches the plot's edges
    const first = Math.max(0, Math.floor(lo / binHz));
    const last = Math.min(bins.length - 1, Math.ceil(hi / binHz));
    ctx.strokeStyle = spectrum.transmitting ? c.tx : c.accent;
    ctx.lineWidth = 1.25;
    ctx.beginPath();
    for (let index = first; index <= last; index++) {
      const at = x(index * binHz);
      if (index === first) ctx.moveTo(at, y(bins[index]));
      else ctx.lineTo(at, y(bins[index]));
    }
    ctx.stroke();
    let peak = first;
    let peakDb = -Infinity;
    for (let index = first; index <= last; index++) {
      if (bins[index] > peakDb) {
        peakDb = bins[index];
        peak = index;
      }
    }
    caption.textContent = spectrum.transmitting
      ? "transmitting — the sound card's own output"
      : `peak ${Math.round(peak * binHz)} Hz at ${peakDb.toFixed(0)} dBFS`;
  }

  function heat(fraction) {
    if (!palette) {
      const c = tokens();
      const stops = PALETTES[waterfall.palette] ?? [rgb(c.plot), rgb(c.accent), [255, 255, 255]];
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

  /// The noise in the modem's channel: the quieter third of the averaged bins inside it —
  /// outside, a radio's filter makes the band quieter than the noise the signals sit in, and a
  /// floor taken there drew all of the channel's noise bright.
  function trackNoise(bins, spectrum) {
    const [lo, hi] = spectrum.passband_hz ?? [300, 2700];
    const inside = bins.filter((_, i) => {
      const hz = i * spectrum.bin_hz;
      return hz >= lo && hz <= hi;
    });
    const sample = inside.length >= 8 ? inside : bins;
    const sorted = [...sample].sort((a, b) => a - b);
    const quiet = sorted[Math.floor(sorted.length / 3)];
    if (noise === null) noise = quiet;
    else noise += (quiet - noise) * (quiet < noise ? NOISE_FALL : NOISE_RISE);
    waterfallFloor = noise + WATERFALL_FLOOR_ABOVE_NOISE_DB;
  }

  /// Each bin's power, averaged over the spectra polled so far (linear, then back to dB).
  function average(bins) {
    if (!averaged || averaged.length !== bins.length) {
      averaged = Float64Array.from(bins, (db) => 10 ** (db / 10));
    } else {
      for (let i = 0; i < bins.length; i++) {
        averaged[i] += (10 ** (bins[i] / 10) - averaged[i]) * WATERFALL_AVERAGE;
      }
    }
    return Array.from(averaged, (power) => 10 * Math.log10(Math.max(power, 1e-14)));
  }

  function drawWaterfall(spectrum, span) {
    const [lo, hi] = span;
    // the band, averaged; while this station transmits, what the sound card is sending
    const bins = spectrum.transmitting ? (spectrum.bins_db ?? []) : (averagedBins ?? spectrum.bins_db ?? []);
    if (bins.length === 0) return;
    // an auto floor with no noise heard yet is its starting −90 dBFS, under any real noise: the
    // first lines drew solid white until the station first listened between transmissions
    if (waterfall.auto && noise === null) return;
    const canvas = $("chart-waterfall");
    const width = Math.max(1, canvas.clientWidth || 600);
    const height = Math.max(1, canvas.clientHeight || 140);
    const respanned = drawnSpan !== null && (drawnSpan[0] !== lo || drawnSpan[1] !== hi);
    if (canvas.width !== width || canvas.height !== height || respanned) {
      // a new size or span keeps what is on screen: each line's frequencies moved to where the
      // new span puts them, the lines as they were (one a pixel), so resizing the window,
      // changing its view or its span does not wipe the minutes being watched
      const kept = document.createElement("canvas");
      kept.width = canvas.width;
      kept.height = canvas.height;
      kept.getContext("2d").drawImage(canvas, 0, 0);
      canvas.width = width;
      canvas.height = height;
      if (kept.width > 0 && kept.height > 0) {
        const [was0, was1] = drawnSpan ?? span;
        const left = ((was0 - lo) / (hi - lo)) * width;
        const right = ((was1 - lo) / (hi - lo)) * width;
        canvas.getContext("2d").drawImage(kept, 0, 0, kept.width, kept.height, left, 0, right - left, kept.height);
      }
    }
    drawnSpan = span;
    const ctx = canvas.getContext("2d");
    // the newest line at the top; everything else moves down one
    ctx.drawImage(canvas, 0, 0, width, height - 1, 0, 1, width, height - 1);
    // the floor follows the noise in the channel (`trackNoise`), so the noise stays dark and a
    // signal stays bright whatever the receive level; unless the operator set it
    if (waterfall.auto && spectrumPolls % 8 === 0) {
      $("wf-floor-value").textContent = `auto ${Math.round(waterfallFloor)} dBFS`;
    }
    const floor = waterfall.auto ? waterfallFloor : waterfall.floor;
    const rangeDb = waterfall.gain;
    const row = ctx.createImageData(width, 1);
    const binHz = spectrum.bin_hz;
    for (let px = 0; px < width; px++) {
      // between two bins, the level in between: a pixel narrower than a bin draws no steps
      const at = Math.min(bins.length - 1, (lo + (px / width) * (hi - lo)) / binHz);
      const below = Math.floor(at);
      const above = Math.min(bins.length - 1, below + 1);
      const db = bins[below] + (bins[above] - bins[below]) * (at - below);
      // the low end bent down, so what sits at the noise stays dark and a signal lifts clear
      const level = Math.max(0, Math.min(1, (db - floor) / rangeDb));
      const [r, g, b] = heat(level ** WATERFALL_GAMMA);
      row.data[px * 4] = r;
      row.data[px * 4 + 1] = g;
      row.data[px * 4 + 2] = b;
      row.data[px * 4 + 3] = 255;
    }
    // this station's transmit edges, painted into every line, so they run down the waterfall
    // as two lines; a move to the other bandwidth shows as a step in them
    const edges = transmitEdges(spectrum);
    if (edges) {
      const [r, g, b] = rgb(tokens().tx);
      for (const hz of edges) {
        const px = Math.round(((hz - lo) / (hi - lo)) * width);
        if (px < 0 || px >= width) continue;
        row.data[px * 4] = r;
        row.data[px * 4 + 1] = g;
        row.data[px * 4 + 2] = b;
      }
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
    if (!frame) {
      caption.textContent = "no frame yet";
      return;
    }
    const name = host.modeName(frame.mode);
    const about =
      `${frame.kind} · mode ${frame.mode}${name ? ` ${name}` : ""} · ` +
      `${Number(frame.snr_db).toFixed(1)} dB · ${frame.decoded ? "decoded" : "not decoded"}`;
    if (points.length === 0) {
      // a tone frame (the floor and the middle kinds) is one tone at a time, detected by
      // energy: it has no constellation to draw, and an empty square read as "nothing heard"
      // (the author, 2026-10-10)
      caption.textContent = `tone frame — no constellation · ${about}`;
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
    caption.textContent = about;
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
