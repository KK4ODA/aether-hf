// SPDX-License-Identifier: MIT OR Apache-2.0
// The manual's screenshots, taken of the real panel with example data.
//
//     node tools/manual/screens.mjs [NAME…]
//
// It serves app/ui (the panel and the undocked signal window) on 127.0.0.1, opens it in a
// headless Google Chrome or Chromium driven over the DevTools protocol, and answers the
// panel's control API in the page itself, from the example station below, instead of a running
// modem: a stand-in WebSocket answers each request as the daemon would, and pushes the events
// a real session pushes (metrics twice a second, frames, state changes, log lines, text). The
// clock is fixed, and the minutes before it are played back at once, so the charts, the log and
// the frames table are full. The pages are the ones in the tree, so a screenshot shows the
// panel as it is now; the data is made up and the same at every run. The shots are taken at
// twice the screen's size into docs/manual/screens/, then quantised to a palette
// (tools/manual/screen_data.py shrink). Run it after changing what the panel shows, look at
// each picture, and commit them with the manual. With names, it takes only those shots.
//
// Needs Node 22 or later (for its WebSocket client), Google Chrome, Chromium or Edge ($CHROME
// names one), and `python -m uv` for the ladder (read from the reference model) and the
// quantising (Pillow, fetched by uv). Nothing else: no npm packages. The pictures depend on
// the fonts installed.

import {spawn, spawnSync} from "node:child_process";
import {createServer} from "node:http";
import {existsSync, mkdirSync, readFileSync} from "node:fs";
import {mkdtemp, readFile, rm, writeFile} from "node:fs/promises";
import {tmpdir} from "node:os";
import {dirname, extname, join, resolve, sep} from "node:path";
import {fileURLToPath} from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const UI = join(ROOT, "app", "ui");
const OUT = join(ROOT, "docs", "manual", "screens");
const TYPES = {".html": "text/html", ".js": "text/javascript", ".css": "text/css",
               ".svg": "image/svg+xml", ".png": "image/png", ".json": "application/json"};

// the moment every shot shows: a Saturday afternoon on 40 m
const NOW = Date.parse("2026-11-14T15:24:00-05:00");
const TIMEZONE = "America/New_York";
const MS = 1000;

// ── what is read from the tree, never typed ──────────────────────────────────────────────────

const VERSION = /^version = "([^"]+)"/m.exec(readFileSync(join(ROOT, "pyproject.toml"), "utf8"))[1];

function python(args) {
  const run = spawnSync("python", ["-m", "uv", "run", "--with", "pillow", "python",
    join(ROOT, "tools", "manual", "screen_data.py"), ...args], {cwd: ROOT, encoding: "utf8"});
  if (run.status !== 0) throw new Error(`screen_data.py ${args[0]} failed:\n${run.stderr}`);
  return run.stdout;
}

const CAPS = JSON.parse(python(["ladder"]));
const OCCUPANCY = JSON.parse(readFileSync(join(ROOT, "core", "aetherd", "data", "occupancy.json"),
  "utf8"));
const RULES = JSON.parse(readFileSync(join(ROOT, "core", "aetherd", "data", "regulatory",
  "us-fcc-part97.json"), "utf8"));

/// Where a station's widest transmission reaches at a bandwidth, audio hertz, by the wider of
/// the two readings of §97.3(a)(8), as the daemon's `transmit_hz` and the rules measure it.
function occupied(hz, top) {
  const air = OCCUPANCY.airs[String(hz)];
  const parts = [...air.rungs.filter((r) => r.rung <= top), ...Object.values(air.control)];
  const lo = Math.min(...parts.map((r) => Math.min(r.power[0], r.spectral[0])));
  const hi = Math.max(...parts.map((r) => Math.max(r.power[1], r.spectral[1])));
  const at = (v) => Math.round((air.centre_hz + v) * 10) / 10;
  return {
    edges: [at(lo), at(hi)],
    power: [at(Math.min(...parts.map((r) => r.power[0]))), at(Math.max(...parts.map((r) => r.power[1])))],
    spectral: [at(Math.min(...parts.map((r) => r.spectral[0]))),
               at(Math.max(...parts.map((r) => r.spectral[1])))],
  };
}

// ── a seeded random source: the same pictures at every run ───────────────────────────────────

function seeded(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function gauss(random) {
  const u = Math.max(1e-12, random());
  return Math.sqrt(-2 * Math.log(u)) * Math.cos(2 * Math.PI * random());
}

const round1 = (v) => Math.round(v * 10) / 10;
const b64 = (text) => Buffer.from(text, "utf8").toString("base64");
const kHz = (hz) => `${(hz / 1000).toFixed(3)} kHz`;

// ── the example station ──────────────────────────────────────────────────────────────────────
//
// Made up from end to end: N0CALL-style callsigns, a grid, places and rigs that point at no
// real station. Nothing here may name a tester, their town or their station.

const ME = "N1CALL";
const PEER = "N2CALL";
const DIAL = 7_101_000;
const SEND_RUNG = 11;
const RECEIVE_RUNG = 12;
const CONFIG_PATH = "C:\\Users\\N1CALL\\AppData\\Roaming\\aether-hf\\station.toml";
const DATA_DIR = "C:\\Users\\N1CALL\\AppData\\Roaming\\aether-hf";

function config({callsign = ME, configured = true, bandwidth = 2300} = {}) {
  return {
    schema_version: 13,
    callsign,
    operator: configured
      ? {grid: "EN52", rig: "FTDX10", power_w: 50, antenna: "Dipole at 30 ft"}
      : {grid: "", rig: "", power_w: null, antenna: ""},
    audio: configured
      ? {input: "Microphone (USB AUDIO  CODEC)", output: "Speakers (USB AUDIO  CODEC)",
         sample_rate: 48000, tx_level: 0.25}
      : {input: null, output: null, sample_rate: 48000, tx_level: 0.25},
    ptt: configured
      ? {kind: "cat", port: "COM6", protocol: "yaesu", baud: 38400, civ_address: null,
         source: "data"}
      : {kind: "none"},
    radio: {max_key_s: 30, wait_for_clear: true, busy_threshold_db: 6, bandwidth,
            answer_only: false, answer_gap_ms: 0, turnaround_ms: 0, max_mode: 19,
            compress: false, cw_id: false, cw_id_wpm: 20, identify_by: "morse"},
    control: {enabled: true, bind: "127.0.0.1:8515", token: null, ui_dir: null},
    host: {enabled: false, bind: "127.0.0.1:8300", trace: false},
    kiss: {enabled: false, bind: "127.0.0.1:8100", rung: 1, wait_for_clear: true,
           max_clients: 4, trace: false},
    log: {format: "text", file: "aetherd.log", keep: 500},
    update: {channel: "beta", check: true},
    record: {dir: null, auto: configured, notes: "", tx_audio: false, send_to_project: true},
    sim: {listen: null, connect: null, snr_db: 30},
    panel: {interface: null,
            waterfall: {auto: true, floor_db: -90, gain_db: 20, speed: 8, palette: "aether"}},
    regulatory: configured
      ? {profile: "us-fcc-part97", control: "local", license_class: "general", sideband: "usb",
         itu_region: 2, edge_margin_hz: 50, band_plan: true, dial_hz: null,
         log_permitted: false}
      : {profile: "", control: "", license_class: "", sideband: "usb", itu_region: 2,
         edge_margin_hz: 50, band_plan: true, dial_hz: null, log_permitted: false},
  };
}

const LIVE_KEYS = ["audio.tx_level", "radio.wait_for_clear", "radio.busy_threshold_db",
  "radio.bandwidth", "radio.answer_only", "radio.answer_gap_ms", "radio.turnaround_ms",
  "radio.max_mode", "radio.cw_id", "radio.cw_id_wpm", "radio.identify_by", "record.auto",
  "record.notes",
  "record.send_to_project", "regulatory.dial_hz", "panel.waterfall.auto"];

const DEVICES = {
  devices: [
    {name: "Microphone (USB AUDIO  CODEC)", input: true, output: false, input_rates: [48000],
     output_rates: []},
    {name: "Speakers (USB AUDIO  CODEC)", input: false, output: true, input_rates: [],
     output_rates: [48000]},
    {name: "Microphone Array (Realtek(R) Audio)", input: true, output: false,
     input_rates: [48000], output_rates: []},
    {name: "Speakers (Realtek(R) Audio)", input: false, output: true, input_rates: [],
     output_rates: [48000]},
  ],
  serial_ports: [
    {name: "COM6", description: "Silicon Labs Dual CP2105 USB to UART Bridge: Enhanced COM Port"},
    {name: "COM7", description: "Silicon Labs Dual CP2105 USB to UART Bridge: Standard COM Port"},
  ],
  gpio_interfaces: [],
};

const MEMORIES = [
  [3_590_000, "80 m — Aether calling (proposed)"],
  [7_101_000, "40 m — Aether calling (proposed)"],
  [7_103_500, "Club net, Sundays 8 pm"],
  [10_141_000, "30 m — Aether calling at 500 Hz (proposed)"],
  [14_107_000, "20 m — Aether calling (proposed)"],
  [18_107_000, "17 m — Aether calling (proposed)"],
  [21_094_000, "15 m — Aether calling (proposed)"],
  [24_926_000, "12 m — Aether calling (proposed)"],
  [28_126_000, "10 m — Aether calling (proposed)"],
  [50_690_000, "6 m — Aether calling (proposed)"],
].map(([hz, name]) => ({hz, name}));

const PROFILES = {
  active: "home",
  name: "Home",
  dirty: false,
  profiles: [
    {id: "home", name: "Home", path: `${DATA_DIR}\\profiles\\home.aetherprofile`,
     modified: "2026-11-12 19:40"},
    {id: "field-day", name: "Field Day", path: `${DATA_DIR}\\profiles\\field-day.aetherprofile`,
     modified: "2026-06-27 09:12"},
    {id: "varac", name: "VarAC", path: `${DATA_DIR}\\profiles\\varac.aetherprofile`,
     modified: "2026-10-30 21:05"},
  ],
  dir: `${DATA_DIR}\\profiles`,
  extension: "aetherprofile",
};

const RADIOPOST = {id: 3, app: "RadioPost client 0.1.0-beta.21", kind: "control",
                   since_ms: NOW - 41 * 60 * MS};

/// The rules' view of the station: the indicator, the ceiling, the widest and narrowest
/// transmissions and the dials where each fits, worked out from the profile's own tables.
function regulatory({configured = true, dial = DIAL, bandwidth = 2300} = {}) {
  const profile = {id: RULES.id, name: RULES.name, authority: RULES.authority,
                   rules_as_of: RULES.rules_as_of, source: RULES.source,
                   bandwidth_reading: RULES.bandwidth.reading};
  if (!configured) {
    return {policy: "unset", profile: null, error: null,
            situation: {dial_hz: null, dial_source: null, sideband: "usb", control: null,
                        license: null, itu_region: 2, margin_hz: 50, band_plan: true,
                        power_w: null},
            direction: "originate", indicator: null, ceiling: null, last: null,
            occupied: null, safe_dials: {widest: [], floor: []}};
  }
  const top = bandwidth === 2300 ? 19 : 14;
  const wide = occupied(bandwidth, top);
  const floor = occupied(bandwidth, 0);
  const [lo, hi] = wide.edges;
  const width = Math.round(hi - lo);
  const name = CAPS[String(bandwidth)].modes[top].name;
  const what = `the ${bandwidth} Hz waveform up to rung ${top} (${name})`;
  const segment = RULES.data_segments.find((s) => s.band === "40 m");
  const indicator = {
    verdict: "legal", code: "permitted", rule: segment.rule,
    summary: `FCC: data permitted — occupies ${width} Hz by the FCC's 26 dB measure, inside the 40 m data segment`,
    detail: `${what} occupies ${width} Hz of audio (${Math.round(lo)}–${Math.round(hi)} Hz), so ` +
      `${kHz(dial + lo)}–${kHz(dial + hi)} on the air with the dial at ${kHz(dial)} (USB): ` +
      `inside the 40 m data segment ${kHz(segment.low_hz)}–${kHz(segment.high_hz)} ` +
      `(${segment.rule}), within General privileges, under local control.`,
    what, kind: "data", direction: "originate", dial_hz: dial, dial_source: "radio",
    sideband: "usb", control: "local", license: "general", audio_low_hz: lo, audio_high_hz: hi,
    rf_low_hz: dial + lo, rf_high_hz: dial + hi, bandwidth_hz: hi - lo, margin_hz: 50,
    band: "40 m", segment: {low_hz: segment.low_hz, high_hz: segment.high_hz},
    segment_rule: segment.rule, automatic_segment: null,
    guidance: "Inside the band plan's 40 m RTTY/data area (7.080–7.125 MHz).", notes: [],
  };
  const privileges = RULES.privileges.general;
  const safe = (edges) => {
    const out = [];
    for (const s of RULES.data_segments) {
      if (s.band === "60 m") continue;
      for (const p of privileges) {
        const a = Math.max(s.low_hz, p.low_hz);
        const b = Math.min(s.high_hz, p.high_hz);
        const low = a + 50 - edges[0];
        const high = b - 50 - edges[1];
        if (b > a && high >= low) {
          out.push({band: s.band, segment: {low_hz: s.low_hz, high_hz: s.high_hz},
                    rule: s.rule, dial_low_hz: low, dial_high_hz: high});
        }
      }
    }
    return out;
  };
  return {
    policy: "rules", profile, error: null,
    situation: {dial_hz: dial, dial_source: "radio", sideband: "usb", control: "local",
                license: "general", itu_region: 2, margin_hz: 50, band_plan: true, power_w: 50},
    direction: "originate",
    indicator,
    ceiling: {rung: top, name, of: top + 1, limit: null},
    last: {t_s: 2210.4, age_s: 3.6,
           decision: {...indicator, what: `a data burst at rung ${SEND_RUNG}`}},
    occupied: {widest: {audio: {power: wide.power, spectral: wide.spectral}, rung: top},
               floor: {audio: {power: floor.power, spectral: floor.spectral}, rung: 0}},
    safe_dials: {widest: safe(wide.edges), floor: safe(floor.edges)},
  };
}

function heardList() {
  const ago = (s) => NOW - s * MS;
  return [
    {callsign: PEER, first_heard_ms: ago(3 * 86400 + 4000), last_heard_ms: ago(1), count: 412,
     snr_db: 11.6, best_snr_db: 16.2, frequency_hz: DIAL, mode: RECEIVE_RUNG,
     activity: "connected", detail: null, connected: true, beacons: 2,
     last_beacon_ms: ago(86400 + 3600), bandwidth_hz: 2300},
    {callsign: "N3CALL", first_heard_ms: ago(5 * 86400), last_heard_ms: ago(9 * 60),
     count: 23, snr_db: 3.8, best_snr_db: 7.5, frequency_hz: DIAL, mode: 1,
     activity: "beacon", detail: null, connected: false, beacons: 6,
     last_beacon_ms: ago(9 * 60), bandwidth_hz: 500},
    {callsign: "N4CALL-10", first_heard_ms: ago(12 * 86400), last_heard_ms: ago(22 * 60),
     count: 96, snr_db: 6.1, best_snr_db: 13.9, frequency_hz: 7_102_500, mode: 9,
     activity: "answering", detail: "N6CALL", connected: true, beacons: 0,
     last_beacon_ms: null, bandwidth_hz: 2300},
    {callsign: "N7CALL", first_heard_ms: ago(2 * 3600), last_heard_ms: ago(41 * 60),
     count: 3, snr_db: -6.3, best_snr_db: -2.0, frequency_hz: DIAL, mode: 0,
     activity: "beacon", detail: null, connected: false, beacons: 3,
     last_beacon_ms: ago(41 * 60), bandwidth_hz: 2300},
    {callsign: "N8CALL/P", first_heard_ms: ago(2 * 3600 + 600), last_heard_ms: ago(2 * 3600),
     count: 2, snr_db: 1.2, best_snr_db: 1.4, frequency_hz: 14_107_000, mode: 1,
     activity: "probing", detail: ME, connected: false, beacons: 0, last_beacon_ms: null,
     bandwidth_hz: 2300},
  ];
}

function sessionList() {
  const at = (s) => NOW - s * MS;
  const entry = (s) => ({bandwidth_hz: 2300, frequency_hz: DIAL, test: false, recording: null,
                         ...s, ended_ms: s.started_ms + s.duration_s * MS});
  return [
    entry({remote: PEER, started_ms: at(86400 + 2500), duration_s: 318, role: "caller",
           bytes_sent: 6211, bytes_acked: 6211, bytes_received: 9874, end: "closed",
           snr_db: 10.8, best_snr_db: 15.1, heard_there_db: 9.2, top_rung_sent: 12,
           top_rung_heard: 14, test: true, recording: "20261113-195312_N1CALL_N2CALL_test"}),
    entry({remote: "N4CALL-10", started_ms: at(2 * 86400 + 7200), duration_s: 184,
           role: "caller", bytes_sent: 2840, bytes_acked: 2840, bytes_received: 12208,
           end: "closed", snr_db: 7.4, best_snr_db: 13.9, heard_there_db: 5.1,
           top_rung_sent: 9, top_rung_heard: 11, frequency_hz: 7_102_500,
           recording: "20261112-140810_N1CALL_N4CALL-10"}),
    entry({remote: "N9CALL", started_ms: at(3 * 86400 + 1500), duration_s: 96, role: "called",
           bytes_sent: 0, bytes_acked: 0, bytes_received: 1450, end: "peer disconnected",
           snr_db: -1.8, best_snr_db: 0.6, heard_there_db: null, top_rung_sent: null,
           top_rung_heard: 4, frequency_hz: 3_590_000, bandwidth_hz: 500}),
    entry({remote: PEER, started_ms: at(3 * 86400 + 4000), duration_s: 402, role: "called",
           bytes_sent: 3120, bytes_acked: 3120, bytes_received: 2268, end: "closed",
           snr_db: 2.4, best_snr_db: 5.0, heard_there_db: 1.9, top_rung_sent: 7,
           top_rung_heard: 8, frequency_hz: 3_590_000}),
  ];
}

// ── the minutes before the shot, as events ───────────────────────────────────────────────────

const SENT = [
  {at: -372, ref: "p1", text: "Good afternoon from EN52. You are +11 here, 40 m is in fine shape.",
   state: "delivered"},
  {at: -301, ref: "p2", text: "Rig here is an FTDX10 at 50 W into a dipole at 30 ft.",
   state: "delivered"},
  {at: -176, ref: "p3", text: "Roster received, thank you. See you at the meeting Thursday.",
   state: "delivered"},
  {at: -22, ref: "p4", text: "QSL on the net time. I'll sign in a few minutes, 73.",
   state: "pending"},
];

const RECEIVED = [
  [-338, "Good afternoon! Copy you +10 here in the valley. FT-891 at 80 W, end-fed wire.\n"],
  [-262, "Weather is clear and 58 F. Field Day planning meeting is Thursday at 7.\n"],
  [-231, "Sending the club roster now, about 4 kB.\n"],
  [-118, "Roster sent. The net is Sunday at 8 pm on 7.1035 as usual.\n"],
  [-51, "Thanks for the contact. 73 de N2CALL.\n"],
];

/// The events of `kind`'s last minutes, in order, and the station's phases (what `status` says
/// from each moment on). `kind` is "session" (connected now), "ended" (a session closed a minute
/// ago), "test" (a Test session on its ladder now) or "idle".
function timeline(kind) {
  const random = seeded(7 + kind.length);
  const events = [];
  const add = (t, event, data) => events.push({t: round1(t), event, data});
  const start = -600;
  const connectAt = kind === "test" ? -226 : -390;
  const endAt = kind === "ended" ? -70 : Infinity;
  const inSession = (t) => kind !== "idle" && t >= connectAt && t < endAt;
  const floor = -38.0;
  const rates = CAPS["2300"].modes;
  // the bursts: this station's (it called, so it runs the slots) and the other's in its slot
  // laid out from the end back, so that the moment of the shot falls in the other station's
  // slot: its frames arriving, the RX lamp lit
  const bursts = [];
  if (kind !== "idle") {
    for (let t = Math.min(endAt - 4, 1.9); t > connectAt + 6.5; ) {
      const rx = 2.4 + random() * 1.4;
      bursts.unshift({from: t - rx, to: t, tx: false});
      t -= rx + 0.35;
      bursts.unshift({from: t - 4.9, to: t, tx: true});
      t -= 4.9 + 0.35;
    }
  }
  const burstAt = (t) => bursts.find((b) => t >= b.from && t < b.to) ?? null;
  // the frames the other station's slots carried, and how this one heard them
  let snr = null;
  let cfo = null;
  let frameT = null;
  let peer = 9.6;
  let rateSnr = null;
  let session = 40;
  let bytesSent = 0;
  let bytesReceived = 0;
  let base = 0;
  let missed = 0;
  for (const b of bursts.filter((x) => !x.tx)) {
    const n = 1 + Math.floor((b.to - b.from - 0.9) / 1.06);
    const count = n;
    snr = round1(11.2 + gauss(random) * 0.9);
    peer = round1(Math.min(11.5, Math.max(8.2, peer + gauss(random) * 0.5)));
    add(b.from + 0.55, "frame", {
      t_s: 0, kind: "control", mode: 6, rv: 0, snr_db: snr, cfo_hz: round1(4 + gauss(random) * 0.6),
      confidence: 1, detect_confidence: round1(2.4 + random()), decoded: true, bytes: 12,
      from: PEER, to: ME, bandwidth_hz: null, follows: null, capture_skip: false, echo: null,
      control: `Answer session ${session} count ${count} level 0 base ${base} bitmap 0x003f ` +
        `missed ${missed} n 6 snr ${Math.round(peer)}`,
    });
    for (let i = 0; i < n; i++) {
      const at = b.from + 0.6 + 1.06 * (i + 1);
      if (at > b.to + 0.4) break;
      const ok = random() > 0.04;
      snr = round1(11.2 + gauss(random) * 0.9);
      cfo = round1(4 + gauss(random) * 0.6);
      frameT = at;
      add(at, "frame", {
        t_s: 0, kind: "data", mode: RECEIVE_RUNG, rv: ok ? 0 : 2, snr_db: snr, cfo_hz: cfo,
        confidence: round1(1.8 + random() * 0.9), detect_confidence: round1(2 + random()),
        decoded: ok, bytes: ok ? rates[RECEIVE_RUNG].payload_bytes : 0, from: PEER, to: ME,
        bandwidth_hz: null, follows: Math.max(0, Math.min(3, n - i - 1)),
        capture_skip: false, echo: null, control: null,
      });
      if (ok) bytesReceived += rates[RECEIVE_RUNG].payload_bytes - 3;
    }
    base = (base + 6) % 256;
    missed = random() > 0.92 ? 1 : 0;
  }
  // twice a second, what the busy detector, the meter and the link say
  let throughput = 0;
  let lastSnr = null;
  let lastCfo = null;
  let lastFrame = null;
  const frames = events.filter((e) => e.event === "frame").sort((a, b) => a.t - b.t);
  let next = 0;
  for (let t = start; t <= 0.001; t += 0.5) {
    while (next < frames.length && frames[next].t <= t) {
      lastSnr = frames[next].data.snr_db;
      lastCfo = frames[next].data.cfo_hz;
      lastFrame = 2200 + frames[next].t;
      next += 1;
    }
    const burst = burstAt(t);
    const rx = burst !== null && !burst.tx;
    const tx = burst !== null && burst.tx;
    const live = inSession(t);
    const noise = gauss(random) * 1.1;
    const level = floor + (rx ? 14.8 + gauss(random) * 1.2 : noise);
    if (live && t > connectAt + 20) {
      const target = 0.36 * (rates[SEND_RUNG].net_bit_rate + rates[RECEIVE_RUNG].net_bit_rate);
      throughput += (target * (0.85 + 0.3 * random()) - throughput) * 0.04;
    } else if (!live) {
      throughput = 0;
    }
    if (live && tx) bytesSent += 0.5 * rates[SEND_RUNG].net_bit_rate / 8 * 0.62;
    if (live && lastSnr !== null) rateSnr = round1(peer - 0.4 + gauss(random) * 0.2);
    const seconds = live ? t - connectAt : null;
    add(t, "metrics", {
      mode: live ? SEND_RUNG : 5,
      queued_bytes: live ? (t > -25 ? 54 : Math.round(random() * 300)) : 0,
      noise_floor_db: round1(floor + gauss(random) * 0.15),
      level_db: round1(level),
      excess_peak_db: round1(Math.abs(level - floor) + Math.abs(gauss(random)) * 1.4),
      shape_db: round1(rx ? 7.6 + random() : 6.2 + random() * 0.8),
      radio_gain_db: null,
      channel_busy: rx || Boolean(burstAt(t - 0.6) && !burstAt(t - 0.6).tx),
      busy_reason: rx ? "frame" : "level",
      transmitting: tx,
      receiving: rx,
      audio: {rms_dbfs: round1((rx ? -24.1 : -28.6) + gauss(random) * 0.4),
              peak_dbfs: round1((rx ? -13.2 : -17.9) + gauss(random) * 0.6), clipping: 0,
              settled: true, advice: "Good."},
      tx_peak_dbfs: -7.9,
      snr_db: lastSnr,
      cfo_hz: lastCfo,
      last_frame_s: lastFrame,
      peer_snr_db: live ? peer : null,
      rate_snr_db: live ? rateSnr : null,
      missed_there: live ? missed : null,
      throughput_bps: Math.round(throughput),
      sending_rung: live && t > connectAt + 3 ? SEND_RUNG : null,
      receiving_rung: live && t > connectAt + 8 ? RECEIVE_RUNG : null,
      link: live ? {started_s: 2200 + connectAt, seconds: round1(seconds), remote: PEER,
                    bytes_sent: Math.round(bytesSent), bytes_received: Math.round(bytesReceived)}
        : null,
    });
    if (tx && !burstAt(t - 0.5)?.tx) add(t, "ptt", {on: true});
    if (!tx && burstAt(t - 0.5)?.tx) add(t, "ptt", {on: false});
  }
  // what the modem says in words
  const log = (t, name, detail) => add(t, "log", {name, detail, state: "Idle", callsign: ME,
                                                  remote: null});
  add(-598, "log", {name: "claims", detail: "RadioPost client 0.1.0-beta.21 is using the station",
                    state: "Idle", callsign: ME, remote: null});
  log(-597, "identifier", "N1CALL identifies in Morse at the end of every session: Required for US callsigns (§97.119)");
  log(-574, "keying", "the radio takes 19 ms to key and a transmission 34 ms to render; transmissions now start 103 ms after they are decided, and are decided that much ahead");
  log(-561, "beacon", "heard N3CALL at 4.2 dB");
  add(-533, "regulatory", {verdict: "blocked", what: "a tuner tone",
    summary: "TX BLOCKED: 7 020.000 kHz is outside the General privileges on 40 m",
    detail: "The tone at 1500 Hz is on the air at 7 021.500 kHz with the dial at 7 020.000 kHz (USB): General privileges on 40 m are 7 025.000–7 125.000 and 7 175.000–7 300.000 kHz (§97.301(d)).",
    rf_low_hz: 7_021_500, rf_high_hz: 7_021_500});
  log(-519, "tune", "dial set to 7101000 Hz");
  log(-488, "beacon", "sent");
  if (kind !== "idle") {
    if (kind !== "test") log(connectAt - 31, "probe", `${PEER} hears us at 9 dB, heard at 11.2 dB`);
    log(connectAt, "recording",
        `started ${DATA_DIR}\\recordings\\20261114-151730_N1CALL_N2CALL.wav`);
  }
  if (kind === "test") {
    log(-244, "test", `started with ${PEER}`);
    log(-236, "probe", `${PEER} hears us at 9 dB, heard at 11.2 dB`);
    log(-227, "test", "connected");
  }
  if (kind !== "idle") {
    add(connectAt, "state", {name: "connected", detail: `${PEER} (master)`, state: "Connected",
                             remote: PEER, callsign: ME, owner: null});
    add(connectAt, "state", {name: "role", detail: "master", state: "Connected", remote: PEER,
                             callsign: ME, owner: null});
  }
  if (kind === "session" || kind === "ended") {
    for (const [t, text] of RECEIVED) if (t < endAt) add(t, "data", {data: b64(text)});
    for (const m of SENT) {
      if (m.state === "delivered" && m.at < endAt) {
        add(m.at + 30, "sent", {ref: m.ref, bytes: m.text.length + 1, delivered: true,
                                reason: null});
      }
    }
  }
  if (kind === "test") {
    log(-171, "test", "message: 1024 bytes in 54 s (152 bit/s)");
    log(-96, "test", "file: 4096 bytes in 74 s (443 bit/s)");
    log(-63, "test", "rung 11: 4/4 decoded at 10 dB");
    log(-49, "test", "rung 12: 4/4 decoded at 10 dB");
    log(-34, "test", "rung 14: 4/4 decoded at 11 dB");
  }
  if (kind === "ended") {
    log(-80, "closing", "disconnecting from N2CALL: nothing left to send");
    add(endAt, "state", {name: "disconnected", detail: "closed", state: "Idle", remote: PEER,
                         callsign: ME, owner: null, identifier_pending: true});
    add(endAt + 6.5, "state", {name: "identified", detail: "the closing identifier went out as N1CALL",
                               state: "Idle", remote: PEER, callsign: ME, owner: null});
    add(endAt + 6.6, "session", {...sessionList()[0], started_ms: NOW + connectAt * MS,
      ended_ms: NOW + endAt * MS, duration_s: endAt - connectAt, test: false,
      recording: "20261114-151730_N1CALL_N2CALL", bytes_sent: 11840, bytes_acked: 11840,
      bytes_received: 9214, top_rung_sent: SEND_RUNG, top_rung_heard: RECEIVE_RUNG,
      snr_db: 11.4, heard_there_db: 9.8});
  }
  events.sort((a, b) => a.t - b.t);
  // what `status` says from each moment on
  const phases = [{t: start, patch: {state: "idle", role: "none", remote: null, session: null}}];
  if (kind !== "idle") {
    phases.push({t: connectAt - 12, patch: {state: "connecting", role: "none", remote: PEER}});
    phases.push({t: connectAt, patch: {state: "connected", role: "master", remote: PEER,
                                       session}});
  }
  if (kind === "ended") {
    phases.push({t: endAt, patch: {state: "idle", role: "none", remote: PEER, session: null}});
  }
  return {start, events, phases, connectAt: kind === "idle" ? null : connectAt, endAt,
          frameT};
}

function constellation(random) {
  // the last data frame: 8-PSK (rung 12), equalised, at about +11 dB
  const points = [];
  for (let i = 0; i < 640; i++) {
    const k = Math.floor(random() * 8);
    const a = (Math.PI / 4) * k + Math.PI / 8;
    const sigma = 0.085;
    points.push([round3(Math.cos(a) + gauss(random) * sigma),
                 round3(Math.sin(a) + gauss(random) * sigma)]);
  }
  return {
    frame: {t_s: 2199.2, kind: "data", mode: RECEIVE_RUNG, rv: 0, snr_db: 11.4, cfo_hz: 4.2,
            confidence: 2.31, detect_confidence: 2.62, decoded: true,
            bytes: CAPS["2300"].modes[RECEIVE_RUNG].payload_bytes, from: PEER, to: ME,
            control: null, bandwidth_hz: null, follows: 0, capture_skip: false, echo: null},
    points,
  };
}

function round3(v) {
  return Math.round(v * 1000) / 1000;
}

// ── the scenes ───────────────────────────────────────────────────────────────────────────────

const SEEN = {"aether.debugNoticeSeen": "1", "aether.chime": "off"};

/// What one shot needs: the page, the window, what the browser keeps, the station's world and
/// what to do before the picture is taken.
function world(scene) {
  const kind = scene.timeline ?? "session";
  const t = timeline(kind);
  const configured = scene.configured !== false;
  const bandwidth = scene.bandwidth ?? 2300;
  const callsign = configured ? ME : "N0CALL";
  const random = seeded(99);
  const test = kind === "test"
    ? {remote: PEER, step: "ladder", elapsed_s: 244, budget_s: 600, remaining_s: 356, rungs: 3,
       transfer: null,
       ladder: {total: 8, done: 3, testing: 15, testing_name: CAPS["2300"].modes[15].name,
                frames: 4, highest_passed: 14, highest_passed_name: CAPS["2300"].modes[14].name,
                failures_in_row: 0, failures_allowed: 3,
                last: {mode: 14, frames: 4, decoded: 4, snr_db: 11.3, seconds: 6.2}},
       link: {rung: RECEIVE_RUNG, rung_name: CAPS["2300"].modes[RECEIVE_RUNG].name,
              heard_there_db: 9.4}}
    : null;
  const pending = kind === "session" ? ["p4"] : [];
  const mismatch = scene.mismatch
    ? {callsign: "N3CALL", what: "call", theirs_hz: 2300, ours_hz: 500, at_ms: NOW - 40 * MS,
       sentence: "N3CALL called this station at 2300 Hz and was not answered: it runs 500 Hz. To work N3CALL, set the bandwidth to 2300 Hz in Setup step 4 (a 2300 Hz station still answers 500 Hz calls), or ask N3CALL to call at 500 Hz."}
    : null;
  const status = {
    frequency_hz: configured ? DIAL : null,
    callsign,
    callsigns: [callsign],
    own_callsigns: [callsign],
    claims: [],
    programs: configured ? [RADIOPOST] : [],
    owner: null,
    mode: SEND_RUNG,
    transmitting: false,
    probing: false,
    probe_try: null,
    closing: false,
    leave_in_s: null,
    channel_busy: false,
    compressing: false,
    compression_saving: 0,
    uptime_s: 2200,
    ptt: configured ? "Yaesu CAT on COM6" : "none (vox or receive only)",
    ptt_fault: null,
    can_tune: configured,
    // a gateway program scanning three dials, the radio on the second (ADR-0129)
    scan: scene.scan
      ? {owner: 5, app: "RadioPost Gateway 0.1.0-beta.24", dials: [3_590_000, DIAL, 10_145_000],
         at_hz: DIAL, dwell_s: 13.514, linger_s: 34.928, scan_s: 28, held: null,
         lingering_until_ms: null, hops: 41, dropped: [], since_ms: NOW - 520 * MS}
      : null,
    queued_bytes: kind === "session" ? 54 : 0,
    version: VERSION,
    recording: kind === "session" || kind === "test"
      ? {path: `${DATA_DIR}\\recordings\\20261114-151730_N1CALL_N2CALL.wav`, seconds: 390,
         marks: 0, meter_readings: 0, reads_meter: true}
      : null,
    test,
    beacon: {waiting: false, waiting_for_clear: false, every_s: null, next_in_s: null,
             last_sent_ms: configured ? NOW - 488 * MS : null, sent: configured ? 1 : 0,
             skipped: 0},
    identifier: {enabled: true, setting: false, forced: configured, callsign,
                 reason: configured ? "Required for US callsigns (§97.119)" : null,
                 sent_as: callsign, wpm: 20, sent_wpm: 20, max_wpm: configured ? 20 : null,
                 pending: false},
    regulatory: regulatory({configured, bandwidth}),
    datagrams: {queued: 0, limit: 16, sent: 0, heard: 0, incomplete_dropped: 0},
    sent: {pending, recent: SENT.filter((m) => m.state === "delivered")
      .map((m) => ({ref: m.ref, bytes: m.text.length + 1, delivered: true, reason: null}))},
    counters: kind === "idle" || !configured
      ? {frames_sent: 0, frames_resent: 0, frames_received: 0, frames_failed: 0,
         harq_rescues: 0, bytes_delivered: 0, silent_slots: 0, forfeits: 0, dropped_late: 0,
         headers_elided: 0, left_answered: 0, transmissions: 0, frames_detected: 0,
         deferred_for_busy: 0, watchdog_trips: 0, beacons_sent: 0, beacons_heard: 0, cw_ids: 0,
         calls_unanswered: 0, probes_sent: 0, probes_answered: 0, probe_replies: 0,
         frames_reencoded: 0, frames_split: 0}
      : {frames_sent: 236, frames_resent: 9, frames_received: 214, frames_failed: 6,
         harq_rescues: 4, bytes_delivered: 18432, silent_slots: 1, forfeits: 0, dropped_late: 0,
         headers_elided: 12, left_answered: 0, transmissions: 61, frames_detected: 263,
         deferred_for_busy: 1, watchdog_trips: 0, beacons_sent: 1, beacons_heard: 5, cw_ids: 2,
         calls_unanswered: 0, probes_sent: 1, probes_answered: 2, probe_replies: 1,
         frames_reencoded: 0, frames_split: 0},
    recordings_dir: `${DATA_DIR}\\recordings`,
    bandwidth: {bandwidth_hz: bandwidth, home_hz: bandwidth, host_hz: null, why: "configured",
                caller: null, callee: null, moves: 0, mismatch},
    answering: true,
    not_answering: null,
    supervised: true,
    host: {enabled: false, connected: false},
    kiss: {listening: false, address: null, error: null, clients: [], enabled: false,
           bind: "127.0.0.1:8100", rung: 1, wait_for_clear: true, ignore_dcd: false},
    audio_fault: null,
    config_note: null,
    upload: {state: "idle", name: null, sent: 0, total: 0, error: null, link: null, to: null},
    debug: {on: true, waiting: [], sending: null, sent: 0, last_error: null},
    binary: "C:\\Users\\N1CALL\\AppData\\Local\\Aether HF\\aetherd.exe",
  };
  const metricsBase = {
    mode: 5, queued_bytes: 0, noise_floor_db: -38, level_db: -38, excess_peak_db: 0.8,
    shape_db: 6.4, radio_gain_db: null, channel_busy: false, busy_reason: null,
    transmitting: false, receiving: false,
    audio: configured
      ? {rms_dbfs: -28.4, peak_dbfs: -17.6, clipping: 0, settled: true, advice: "Good."}
      : {rms_dbfs: -61.0, peak_dbfs: -48.2, clipping: 0, settled: true,
         advice: "Very quiet. Turn the radio's audio output or the sound card's input up."},
    tx_peak_dbfs: configured ? -7.9 : null, snr_db: null, cfo_hz: null, last_frame_s: null,
    peer_snr_db: null, rate_snr_db: null, missed_there: null, throughput_bps: 0,
    sending_rung: null, receiving_rung: null,
    rx_filter: scene.filter ?? (configured
      ? {verdict: "clear", low_drop_db: 2.4, high_drop_db: 3.1, looks: 120}
      : {verdict: "waiting", low_drop_db: null, high_drop_db: null, looks: 5}),
    clipping: {share: 0, clips: false},
    link: null,
  };
  const caps = CAPS[String(bandwidth)];
  const wide = occupied(bandwidth, bandwidth === 2300 ? 19 : 14);
  const nominal = OCCUPANCY.airs[String(bandwidth)].nominal_hz;
  return {
    now: NOW,
    start: scene.replay === false ? 0 : -t.start,
    timeline: scene.replay === false ? [] : t.events,
    phases: t.phases,
    connectAt: t.connectAt,
    endAt: Number.isFinite(t.endAt) ? t.endAt : null,
    status,
    metricsBase,
    caps,
    config: config({callsign, configured, bandwidth}),
    configPath: CONFIG_PATH,
    devices: configured ? DEVICES : {...DEVICES, serial_ports: DEVICES.serial_ports},
    memories: MEMORIES,
    profiles: configured ? PROFILES
      : {active: "default", name: "Default", dirty: false,
         profiles: [{id: "default", name: "Default",
                     path: `${DATA_DIR}\\profiles\\default.aetherprofile`, modified: null}],
         dir: `${DATA_DIR}\\profiles`, extension: "aetherprofile"},
    heard: configured ? heardList() : [],
    sessions: configured ? sessionList() : [],
    constellation: configured ? constellation(random) : {frame: null, points: []},
    spectrum: {transmit_hz: wide.edges, passband_hz: [1500 - bandwidth / 2, 1500 + bandwidth / 2],
               // the span the other station's signal fills: the air's nominal width, centred
               signal_hz: [1500 - nominal / 2, 1500 + nominal / 2],
               signal: configured && kind !== "idle" && scene.signal !== false},
    version: VERSION,
    storage: {
      ...(configured ? SEEN : {"aether.chime": "off"}),
      "aether.sent": JSON.stringify(kind === "session" || kind === "ended"
        ? SENT.filter((m) => kind === "session" || m.state === "delivered")
          .map((m) => ({ref: m.ref, at: NOW + m.at * MS, to: PEER, text: m.text,
                        bytes: m.text.length + 1, state: m.state}))
        : []),
      ...(scene.storage ?? {}),
    },
    page: scene.page ?? "index.html",
    peer: PEER,
    seed: 4242,
  };
}

/// The stand-in modem, run in the page before its own scripts: the clock, the store, the
/// shell's few calls, and a WebSocket that answers the control API from `D`. Serialised into
/// the page, so it may use nothing outside itself.
function pageWorld(D) {
  try {
    localStorage.clear();
    for (const [key, value] of Object.entries(D.storage)) localStorage.setItem(key, value);
    sessionStorage.setItem("aether-splashed", "1");
    sessionStorage.setItem("aether-fitted", "1");
  } catch {
    // a page that keeps nothing: the panel's defaults
  }
  const RealDate = Date;
  const S = (window.__screens = {ready: false, t: -D.start, wanted: null, errors: []});
  const nowMs = () => D.now + S.t * 1000;
  class FakeDate extends RealDate {
    constructor(...args) {
      if (args.length === 0) super(nowMs());
      else super(...args);
    }
    static now() {
      return nowMs();
    }
  }
  window.Date = FakeDate;

  let seed = D.seed >>> 0;
  const random = () => {
    seed = (seed + 0x6d2b79f5) >>> 0;
    let t = seed;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };

  // the desktop shell's part, as far as the panel asks it: the updater's view for Help /
  // About, a window to size (the small views say what size they want), and nothing else
  const listeners = new Map();
  class LogicalSize {
    constructor(width, height) {
      this.width = width;
      this.height = height;
    }
  }
  const win = {
    isMaximized: async () => false,
    scaleFactor: async () => 2,
    innerSize: async () => ({width: window.innerWidth * 2, height: window.innerHeight * 2}),
    setSize: async (size) => {
      S.wanted = {width: size.width, height: size.height};
    },
    setMinSize: async () => {},
    setAlwaysOnTop: async () => {},
    setTheme: async () => {},
  };
  window.__TAURI__ = {
    event: {
      listen: async (name, callback) => {
        listeners.set(name, callback);
        return () => listeners.delete(name);
      },
      emit: async (name, payload) => {
        if (name === "panel-update" && payload?.action === "view") {
          setTimeout(() => listeners.get("update")?.({payload: {
            phase: "up-to-date", checked: true, current: D.version, channel: "beta"}}), 0);
        }
      },
    },
    window: {getCurrentWindow: () => win, LogicalSize},
    dpi: {LogicalSize},
  };

  // what the status says now: the station's base, the phase it is in, the last metrics
  let lastMetrics = {...D.metricsBase};
  function status() {
    const out = structuredClone(D.status);
    for (const phase of D.phases) if (phase.t <= S.t) Object.assign(out, phase.patch);
    out.metrics = lastMetrics;
    out.link = lastMetrics.link;
    out.transmitting = lastMetrics.transmitting;
    out.channel_busy = lastMetrics.channel_busy;
    if (out.state === "idle") {
      out.test = null;
      out.queued_bytes = 0;
    }
    out.uptime_s = 2200 + S.t;
    return out;
  }

  // what the radio hears, as the daemon's `spectrum` reports it (the last window of captured
  // audio): the band's noise through the radio's ~300–2700 Hz filter; in the other station's
  // slot its signal, a plateau across the span it occupies
  // about 13 dB over the noise in each bin (12.3 dB in 3 kHz), with a slow fade across it; in
  // this station's own slot next to nothing, the receiver muted while it transmits. The slots
  // follow a session's rhythm from the page's start, seeded: this station's header and data
  // 5–10 s, a gap of band noise, the other's answer and data 3–8 s, a gap.
  const BIN = 48000 / 4096;
  const BINS = Math.floor(4000 / BIN) + 1;
  const NOISE_DB = -88;
  const SIGNAL_DB = NOISE_DB + 13;
  const loaded = RealDate.now();
  const slots = [];
  {
    let r = 17;
    const draw = () => {
      r = (r * 1103515245 + 12345) % 2147483648;
      return r / 2147483648;
    };
    for (let t = 0; t < 900; ) {
      for (const [state, least, most] of [["tx", 5, 10], ["idle", 0.3, 0.8], ["rx", 3, 8],
                                          ["idle", 0.3, 0.8]]) {
        const length = least + (most - least) * draw();
        slots.push({from: t, to: t + length, state});
        t += length;
      }
    }
  }
  function spectrum() {
    const s = (RealDate.now() - loaded) / 1000;
    const slot = slots.find((x) => s >= x.from && s < x.to) ?? {state: "idle"};
    const state = !D.spectrum.signal ? "idle" : S.hold ?? slot.state;
    const [lo, hi] = D.spectrum.signal_hz;
    const bins = [];
    for (let i = 0; i < BINS; i++) {
      const f = i * BIN;
      if (state === "tx") {
        bins.push(-116 + random() * 3);
        continue;
      }
      // the radio's filter: flat 300–2700 Hz, falling either side to the sound card's floor
      const roll = f < 300 ? (300 - f) / 4 : f > 2700 ? (f - 2700) / 4 : 0;
      let power = 10 ** ((NOISE_DB - roll) / 10) * -Math.log(1 - random() * 0.999);
      if (state === "rx") {
        // a slow fade across the band and through the burst, a couple of decibels either way
        const fade = 2.2 * Math.sin(2 * Math.PI * (f / 820 + s / 3.7))
          + 1.3 * Math.sin(2 * Math.PI * (f / 260 - s / 1.9));
        const skirt = f < lo ? (lo - f) / 2.5 : f > hi ? (f - hi) / 2.5 : 0;
        power += 10 ** ((SIGNAL_DB + fade - skirt - roll) / 10) * -Math.log(1 - random() * 0.999);
      }
      bins.push(Math.max(-118, 10 * Math.log10(power + 1e-14)));
    }
    return {bin_hz: BIN, bins_db: bins, passband_hz: D.spectrum.passband_hz,
            transmit_hz: D.spectrum.transmit_hz, transmitting: state === "tx"};
  }

  function answer(method, params) {
    switch (method) {
      case "status":
        return status();
      case "capabilities":
        return D.caps;
      case "heard.list":
        return {stations: D.heard, limit: 200, path: "heard.json"};
      case "sessions.list": {
        const kept = D.sessions.filter((s) => s.ended_ms <= nowMs());
        return {sessions: params.remote ? kept.filter((s) => s.remote === params.remote) : kept,
                limit: 500, path: "sessions.json"};
      }
      case "frequencies.list":
        return {memories: D.memories, limit: 200, path: "frequencies.json"};
      case "devices.list":
        return D.devices;
      case "config.get":
        return {config: D.config, path: D.configPath, live_keys: []};
      case "profile.list":
        return D.profiles;
      case "identifier.check": {
        const call = String(params.callsign ?? "").toUpperCase();
        const us = /^(A[A-L]|[KNW])/.test(call) && call !== "N0CALL";
        return {enabled: us, setting: false, forced: us,
                reason: us ? "Required for US callsigns (§97.119)" : null,
                sent_as: call.replace(/-.*$/, ""), callsign: call, wpm: 20, sent_wpm: us ? 20 : null,
                max_wpm: 20};
      }
      case "spectrum":
        return spectrum();
      case "constellation":
        return D.constellation;
      case "test.status":
        return {running: Boolean(D.status.test), results: null};
      default:
        return {};
    }
  }

  const sockets = [];
  class FakeSocket extends EventTarget {
    constructor(url) {
      super();
      this.url = url;
      this.readyState = 0;
      sockets.push(this);
      setTimeout(() => {
        this.readyState = 1;
        this.dispatchEvent(new Event("open"));
        if (sockets.length === 1) begin();
      }, 0);
    }

    send(text) {
      const request = JSON.parse(text);
      const result = answer(request.method, request.params ?? {});
      setTimeout(() => this.deliver({id: request.id, ok: true, result}), 0);
    }

    deliver(frame) {
      if (this.readyState !== 1) return;
      this.dispatchEvent(new MessageEvent("message", {data: JSON.stringify(frame)}));
    }

    close() {
      this.readyState = 3;
    }
  }
  for (const [name, value] of Object.entries({CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3})) {
    FakeSocket[name] = value;
    FakeSocket.prototype[name] = value;
  }
  window.WebSocket = FakeSocket;

  const push = (event, data) => {
    for (const socket of sockets) socket.deliver({event, data});
  };

  // the minutes before the shot, at once: the panel draws them as it would have live
  function begin() {
    setTimeout(() => {
      for (const e of D.timeline) {
        S.t = e.t;
        if (e.event === "metrics") {
          lastMetrics = {...D.metricsBase, ...e.data};
          push("metrics", lastMetrics);
        } else {
          push(e.event, e.data);
        }
      }
      S.t = 0;
      push("metrics", lastMetrics);
      setTimeout(() => {
        S.ready = true;
      }, 700);
    }, 600);
    // a metrics event twice a second from then on, as the daemon sends them
    setInterval(() => {
      if (S.ready) push("metrics", lastMetrics);
    }, 500);
  }

  window.addEventListener("error", (event) => S.errors.push(String(event.message)));
  window.addEventListener("unhandledrejection", (event) => S.errors.push(String(event.reason)));
}

// ── the shots ────────────────────────────────────────────────────────────────────────────────

const WIDTH = 1440;
const HEIGHT = 900;
// the small views' widths, read from app.js, which asks the shell for them
const VIEW_WIDTH = Object.fromEntries([...(/const VIEW_WIDTH = \{([^}]*)\}/
  .exec(readFileSync(join(UI, "app.js"), "utf8"))[1].matchAll(/(\w+):\s*(\d+)/g))]
  .map(([, view, px]) => [view, Number(px)]));

const tab = (name) => `document.getElementById("tab-${name}").click()`;

const SCENES = {
  "first-start": {configured: false, timeline: "idle", replay: false},
  window: {after: [tab("session")]},
  status: {},
  setup: {timeline: "ended", after: [tab("setup"), "hideEndedBanner()"]},
  "setup-modem": {timeline: "ended", after: [tab("setup"), "hideEndedBanner()",
    `document.querySelector('.step[data-step="4"]').scrollIntoView({block: "start"})`]},
  "session-test": {timeline: "test", after: [tab("session")]},
  drive: {timeline: "ended", storage: {"aether.keying.open": "1"},
          after: [tab("session"), "hideEndedBanner()"]},
  stations: {after: [tab("stations")]},
  diagnostics: {after: [tab("diagnostics")], height: 1180, settle: 27000, hold: "rx"},
  log: {timeline: "ended", after: [tab("log")]},
  share: {timeline: "ended", after: [tab("log"), "hideEndedBanner()",
    `document.getElementById("btn-share-open").click()`]},
  help: {timeline: "ended", after: [tab("help"), "hideEndedBanner()"]},
  compact: {storage: {"aether.view": "compact"}, view: "compact"},
  mini: {storage: {"aether.view": "mini"}, view: "mini"},
  light: {storage: {"aether.theme": "light"}, after: [tab("session")]},
  signal: {page: "signal.html", width: 860, height: 640, settle: 52000, hold: "rx"},
  scanning: {timeline: "idle", scan: true, after: [tab("session")], clip: ["#op-card"]},
  banners: {timeline: "idle", bandwidth: 500, mismatch: true, signal: false,
            filter: {verdict: "top", low_drop_db: 3.4, high_drop_db: 17.6, looks: 120},
            clip: ["#mismatch-banner", "#filter-banner"]},
};

// the ended banner comes down by itself 15 s after a session ends: the shot is of the panel
// after it has
const HELPERS = `
  window.hideEndedBanner = () => {
    const banner = document.getElementById("session-banner");
    if (banner.dataset.state === "ended") banner.hidden = true;
  };`;

// ── serving the panel ────────────────────────────────────────────────────────────────────────

function serve() {
  const server = createServer(async (request, response) => {
    const path = decodeURIComponent(new URL(request.url, "http://x").pathname);
    const file = resolve(UI, `.${path === "/" ? "/index.html" : path}`);
    if (!file.startsWith(UI + sep) || !existsSync(file)) {
      response.writeHead(404).end();
      return;
    }
    response.writeHead(200, {"content-type": TYPES[extname(file)] ?? "application/octet-stream",
                             "cache-control": "no-store"});
    response.end(await readFile(file));
  });
  return new Promise((done) => server.listen(0, "127.0.0.1", () => done(server)));
}

// ── Chrome over the DevTools protocol ────────────────────────────────────────────────────────

function chromePath() {
  if (process.env.CHROME) return process.env.CHROME;
  const names = process.platform === "win32"
    ? [`${process.env.PROGRAMFILES}\\Google\\Chrome\\Application\\chrome.exe`,
       `${process.env["PROGRAMFILES(X86)"]}\\Google\\Chrome\\Application\\chrome.exe`,
       `${process.env.LOCALAPPDATA}\\Google\\Chrome\\Application\\chrome.exe`,
       `${process.env["PROGRAMFILES(X86)"]}\\Microsoft\\Edge\\Application\\msedge.exe`]
    : process.platform === "darwin"
      ? ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
         "/Applications/Chromium.app/Contents/MacOS/Chromium"]
      : ["/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/usr/bin/chromium",
         "/usr/bin/chromium-browser", "/snap/bin/chromium"];
  const found = names.find((name) => name && existsSync(name));
  if (!found) throw new Error("no Chrome or Chromium found: set $CHROME");
  return found;
}

async function launch() {
  const profile = await mkdtemp(join(tmpdir(), "aether-screens-"));
  const chrome = spawn(chromePath(), [
    "--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--no-first-run", "--no-default-browser-check", "--hide-scrollbars", "--mute-audio",
    "--force-color-profile=srgb", "--disable-background-timer-throttling",
    "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows",
    "about:blank",
  ], {stdio: ["ignore", "ignore", "pipe"]});
  const url = await new Promise((done, fail) => {
    let said = "";
    chrome.stderr.on("data", (chunk) => {
      said += chunk;
      const m = /DevTools listening on (ws:\/\/\S+)/.exec(said);
      if (m) done(m[1]);
    });
    chrome.on("exit", () => fail(new Error(`Chrome exited:\n${said}`)));
  });
  return {chrome, profile, url};
}

class Cdp {
  constructor(url) {
    this.socket = new WebSocket(url);
    this.next = 1;
    this.waiting = new Map();
    this.handlers = [];
    this.socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id && this.waiting.has(message.id)) {
        const {done, fail} = this.waiting.get(message.id);
        this.waiting.delete(message.id);
        if (message.error) fail(new Error(`${message.error.message} ${message.error.data ?? ""}`));
        else done(message.result);
      } else if (message.method) {
        for (const handler of this.handlers) handler(message);
      }
    });
  }

  open() {
    return new Promise((done) => this.socket.addEventListener("open", done, {once: true}));
  }

  send(method, params = {}, sessionId = undefined) {
    const id = this.next++;
    this.socket.send(JSON.stringify({id, method, params, sessionId}));
    return new Promise((done, fail) => this.waiting.set(id, {done, fail}));
  }

  on(handler) {
    this.handlers.push(handler);
  }
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

async function shoot(cdp, origin, name, scene) {
  const width = scene.width ?? WIDTH;
  const height = scene.height ?? HEIGHT;
  const data = world(scene);
  const {targetId} = await cdp.send("Target.createTarget", {url: "about:blank"});
  const {sessionId} = await cdp.send("Target.attachToTarget", {targetId, flatten: true});
  const send = (method, params) => cdp.send(method, params, sessionId);
  const problems = [];
  cdp.on((message) => {
    if (message.sessionId !== sessionId) return;
    if (message.method === "Runtime.exceptionThrown") {
      problems.push(message.params.exceptionDetails?.exception?.description
        ?? message.params.exceptionDetails?.text);
    } else if (message.method === "Runtime.consoleAPICalled"
               && ["error", "warning", "assert"].includes(message.params.type)) {
      problems.push(message.params.args.map((a) => a.value ?? a.description).join(" "));
    } else if (message.method === "Log.entryAdded" && message.params.entry.level === "error") {
      problems.push(`${message.params.entry.text} ${message.params.entry.url ?? ""}`);
    }
  });
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Log.enable");
  await send("Emulation.setTimezoneOverride", {timezoneId: TIMEZONE});
  await send("Emulation.setLocaleOverride", {locale: "en-US"});
  await send("Emulation.setFocusEmulationEnabled", {enabled: true});
  await send("Emulation.setDeviceMetricsOverride",
             {width, height, deviceScaleFactor: 2, mobile: false});
  await send("Page.addScriptToEvaluateOnNewDocument",
             {source: `(${pageWorld})(${JSON.stringify(data)});${HELPERS}`});
  await send("Page.navigate", {url: `${origin}/${data.page}`});
  const evaluate = async (expression) => {
    const result = await send("Runtime.evaluate",
                              {expression, awaitPromise: true, returnByValue: true});
    if (result.exceptionDetails) {
      throw new Error(`${name}: ${expression}: ${result.exceptionDetails.exception?.description}`);
    }
    return result.result.value;
  };
  for (let i = 0; i < 200; i++) {
    await sleep(100);
    try {
      if (await evaluate("window.__screens?.ready === true")) break;
    } catch {
      // the page is still loading
    }
  }
  await evaluate("document.fonts.ready.then(() => true)");
  if (data.page === "index.html") {
    // what an operator's panel shows: the daemon's own port, not this server's, and the other
    // station's callsign typed in Call
    await evaluate(`document.getElementById("footer-endpoint").textContent =
      "ws://127.0.0.1:8515/v1"; document.getElementById("remote").value = "${PEER}"; true`);
  }
  for (const step of scene.after ?? []) {
    await evaluate(step);
    await sleep(250);
  }
  if (scene.view) {
    // the size the small view asks the desktop shell for, as it would have it: measured again
    // at the view's own width, now the session is up and its banner shows
    await send("Emulation.setDeviceMetricsOverride", {width: VIEW_WIDTH[scene.view],
      height, deviceScaleFactor: 2, mobile: false});
    await sleep(300);
    await evaluate(`document.querySelector('#view-switch [data-view="${scene.view}"]').click()`);
    await sleep(600);
    const wanted = await evaluate("window.__screens.wanted");
    if (wanted) {
      await send("Emulation.setDeviceMetricsOverride", {width: Math.round(wanted.width),
        height: Math.round(wanted.height), deviceScaleFactor: 2, mobile: false});
    }
  }
  await sleep(scene.settle ?? 1500);
  if (scene.hold) {
    // the other station's burst arriving as the picture is taken: the spectrum shows it
    await evaluate(`window.__screens.hold = "${scene.hold}"`);
    await sleep(800);
  }
  await evaluate("document.activeElement?.blur?.(); true");
  let clip;
  if (scene.clip) {
    const bottom = await evaluate(`Math.max(...${JSON.stringify(scene.clip)}.map((s) => {
      const el = document.querySelector(s);
      return el && !el.hidden ? el.getBoundingClientRect().bottom : 0;
    }))`);
    clip = {x: 0, y: 0, width, height: Math.ceil(bottom + 10), scale: 1};
  }
  const errors = await evaluate("window.__screens.errors");
  const shot = await send("Page.captureScreenshot", {format: "png", clip,
                                                     captureBeyondViewport: false});
  const file = join(OUT, `${name}.png`);
  await writeFile(file, Buffer.from(shot.data, "base64"));
  await cdp.send("Target.closeTarget", {targetId});
  for (const problem of [...problems, ...errors]) console.error(`  ${name}: ${problem}`);
  return file;
}

async function main() {
  const wanted = process.argv.slice(2);
  const unknown = wanted.filter((name) => !(name in SCENES));
  if (unknown.length) throw new Error(`no such shot: ${unknown.join(", ")}`);
  const names = wanted.length ? wanted : Object.keys(SCENES);
  mkdirSync(OUT, {recursive: true});
  const server = await serve();
  const origin = `http://127.0.0.1:${server.address().port}`;
  const {chrome, profile, url} = await launch();
  const cdp = new Cdp(url);
  await cdp.open();
  const files = [];
  try {
    for (const name of names) {
      files.push(await shoot(cdp, origin, name, SCENES[name]));
    }
  } finally {
    chrome.kill();
    server.close();
    await sleep(500);
    await rm(profile, {recursive: true, force: true}).catch(() => {});
  }
  process.stdout.write(python(["shrink", ...files]));
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
