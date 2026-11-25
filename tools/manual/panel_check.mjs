// SPDX-License-Identifier: MIT OR Apache-2.0
// The panel's own checks: the real panel in headless Chrome, fed the manual's made-up station
// (screens.mjs), and what an operator reads asserted rather than photographed.
//
//     node tools/manual/panel_check.mjs
//
// Each check opens one of the manual's scenes and asks the page for the problems it finds; any
// problem fails the run. They hold what report 22 changed (2026-11-25): the settings an external
// program sets greyed beside what is in use, the Mode tile's two rows, the speed chart's lines
// both ways, and the banner for a held DISCONNECT. Needs what screens.mjs needs: Node 22 or
// later, Chrome or Chromium ($CHROME names one) and uv.

import {EXTERNAL_CALL, ME, SCENES, Cdp, launch, openScene, serve, sleep} from "./screens.mjs";
import {rm} from "node:fs/promises";

/// The held DISCONNECT in a small view (KK4ODA, 2026-11-25: not seen in Compact): shown, on
/// screen and under the header's display, not over it; its × hides it, and a new held
/// DISCONNECT (another time) shows it again. The status is read every 5 s, so it waits 6.
const HELD_COMPACT = `(async () => {
  const out = [], $ = (id) => document.getElementById(id);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const held = $("held-banner"), x = $("btn-held-dismiss");
  if (!document.body.classList.contains("compact")) out.push("the page is not in a small view");
  const box = held.getBoundingClientRect();
  if (held.hidden || box.height === 0) out.push("the held banner is hidden");
  if (box.bottom > innerHeight) out.push("the held banner ends at " + Math.round(box.bottom) + " px, under the window's " + innerHeight);
  const well = document.querySelector(".well-row").getBoundingClientRect();
  if (box.top < well.bottom - 1) out.push("the held banner starts at " + Math.round(box.top) + " px, over the display well ending at " + Math.round(well.bottom));
  if (!x || x.tagName !== "BUTTON") return out.concat(["the held banner has no dismiss button"]);
  if (x.getAttribute("aria-label") !== "Dismiss" || !x.title) out.push("its × has no name or tooltip");
  const xb = x.getBoundingClientRect();
  if (xb.width === 0 || xb.right > innerWidth) out.push("its × is not on screen");
  x.click();
  await sleep(6000);
  if (!held.hidden) out.push("the held banner came back after its × for the same disconnect");
  if ($("session-banner").hidden) out.push("the session banner went with it");
  window.__screens.held = Date.now();
  await sleep(6000);
  if (held.hidden) out.push("a new held disconnect does not show after the last one was dismissed");
  return out;
})()`;

/// Problems found by an expression run in the page: each returns a list of sentences.
const CHECKS = [
  {
    scene: "setup-external",
    what: "an external program's callsign and bandwidth: Aether's own greyed, what is in use beside them",
    page: `(() => {
      const out = [], $ = (id) => document.getElementById(id);
      const call = $("wz-call"), callNote = $("wz-call-note");
      if (!call.classList.contains("in-use-elsewhere")) out.push("the callsign is not greyed");
      if (call.value !== ${JSON.stringify(ME)})
        out.push("the callsign shows " + call.value + ", not Aether's own");
      if (!callNote.textContent.trim().startsWith(${JSON.stringify(`In use: ${EXTERNAL_CALL}`)}))
        out.push("the callsign's note reads '" + callNote.textContent.trim() + "'");
      const bw = $("radio-bandwidth"), bwNote = $("radio-bandwidth-note");
      if (!bw.classList.contains("in-use-elsewhere")) out.push("the bandwidth is not greyed");
      if (bw.value !== "500") out.push("the bandwidth shows " + bw.value + ", not Aether's own 500");
      if (bwNote.hidden || !bwNote.textContent.trim().startsWith("In use: 2300 Hz"))
        out.push("the bandwidth's note reads '" + bwNote.textContent.trim() + "'");
      if (!$("bw-chip").textContent.includes("external"))
        out.push("the header's chip reads '" + $("bw-chip").textContent.trim() + "'");
      return out;
    })()`,
  },
  {
    scene: "setup",
    what: "no external program: nothing greyed, the callsign's note says what it is for",
    page: `(() => {
      const out = [], $ = (id) => document.getElementById(id);
      if ($("wz-call").classList.contains("in-use-elsewhere")) out.push("the callsign is greyed");
      if ($("wz-call-note").textContent.trim() !== "(for Aether standalone use)")
        out.push("the callsign's note reads '" + $("wz-call-note").textContent.trim() + "'");
      if ($("radio-bandwidth").classList.contains("in-use-elsewhere"))
        out.push("the bandwidth is greyed");
      const note = $("radio-bandwidth-note");
      if (!note.hidden && note.textContent.trim()) out.push("the bandwidth has a note: " + note.textContent);
      return out;
    })()`,
  },
  {
    scene: "status",
    what: "a session both ways: the Mode tile's two rows and the speed chart's two lines",
    page: `(() => {
      const out = [], $ = (id) => document.getElementById(id);
      const labels = [...document.querySelectorAll("#mode-tile .mode-rows .dir")]
        .map((e) => e.textContent.trim());
      if (labels.join("|") !== "Sending|Receiving") out.push("the Mode tile's rows are " + labels.join(", "));
      for (const id of ["v-mode-send", "v-mode-recv"]) {
        if (!/^\\d+$/.test($(id).textContent.trim())) out.push(id + " reads '" + $(id).textContent + "'");
      }
      // the lines over the latest minutes: pixels near each way's state colour in the plot's
      // right quarter, clear of the legend's swatches (which sit top left, as high as the lines)
      const canvas = $("chart-speed");
      const ctx = canvas.getContext("2d");
      const {width, height} = canvas;
      const left = Math.round(width * 0.75);
      const pixels = ctx.getImageData(left, 0, width - left, height).data;
      const style = getComputedStyle(document.body);
      const rgb = (name) => {
        const probe = document.createElement("span");
        probe.style.color = style.getPropertyValue(name).trim();
        document.body.append(probe);
        const m = getComputedStyle(probe).color.match(/\\d+(\\.\\d+)?/g).map(Number);
        probe.remove();
        return m.slice(0, 3);
      };
      const near = ([r, g, b]) => {
        let n = 0;
        for (let i = 0; i < pixels.length; i += 4) {
          if (pixels[i + 3] > 100 && Math.abs(pixels[i] - r) < 40 && Math.abs(pixels[i + 1] - g) < 40
              && Math.abs(pixels[i + 2] - b) < 40) n++;
        }
        return n;
      };
      const tx = near(rgb("--status-tx")), rx = near(rgb("--status-rx"));
      if (tx < 100) out.push("the 'sending at' line has " + tx + " pixels in the plot");
      if (rx < 100) out.push("the 'receiving at' line has " + rx + " pixels in the plot");
      return out;
    })()`,
  },
  {
    scene: "status",
    what: "a session with nothing moving: Sending's next rung in the tile's numerals, no state colour; a dash after it",
    page: `(async () => {
      const out = [], $ = (id) => document.getElementById(id);
      const look = (id) => {
        const c = getComputedStyle($(id));
        return [c.fontFamily, c.fontWeight, c.fontSize, c.fontStyle, c.fontVariantNumeric, c.color].join(" | ");
      };
      const sending = look("v-mode-send");
      window.__screens.metrics({sending_rung: null, receiving_rung: null, mode: 3});
      await new Promise((done) => setTimeout(done, 300));
      const cell = $("v-mode-send"), name = $("v-mode-send-name");
      if (cell.textContent.trim() !== "3") out.push("in a session, Sending reads '" + cell.textContent + "', not the rung alone");
      if (!name.textContent.startsWith("next · ")) out.push("its name reads '" + name.textContent + "'");
      if (look("v-mode-send") !== sending)
        out.push("the next rung is drawn '" + look("v-mode-send") + "', the sending rung '" + sending + "'");
      const style = getComputedStyle(document.body);
      const probe = document.createElement("span");
      document.body.append(probe);
      const colours = ["--status-tx", "--status-rx", "--status-busy", "--status-warning", "--status-error", "--status-link"]
        .map((t) => style.getPropertyValue(t).trim()).filter(Boolean).map((v) => {
          probe.style.color = v;
          return getComputedStyle(probe).color;
        });
      probe.remove();
      for (const id of ["v-mode-send", "v-mode-send-name"]) {
        if (colours.includes(getComputedStyle($(id)).color)) out.push(id + " is drawn in a state colour");
      }
      window.__screens.metrics({link: null});
      await new Promise((done) => setTimeout(done, 300));
      if (cell.textContent.trim() !== "—") out.push("after the session, Sending reads '" + cell.textContent + "'");
      if (name.textContent) out.push("after the session, its name reads '" + name.textContent + "'");
      return out;
    })()`,
  },
  {
    scene: "status",
    with: {storage: {"aether.theme": "light"}},
    what: "the light theme, a session with nothing moving: Sending's next rung in the tile's numerals, no state colour; a dash after it",
    page: `(async () => {
      const out = [], $ = (id) => document.getElementById(id);
      const look = (id) => {
        const c = getComputedStyle($(id));
        return [c.fontFamily, c.fontWeight, c.fontSize, c.fontStyle, c.fontVariantNumeric, c.color].join(" | ");
      };
      const sending = look("v-mode-send");
      window.__screens.metrics({sending_rung: null, receiving_rung: null, mode: 3});
      await new Promise((done) => setTimeout(done, 300));
      const cell = $("v-mode-send"), name = $("v-mode-send-name");
      if (cell.textContent.trim() !== "3") out.push("in a session, Sending reads '" + cell.textContent + "', not the rung alone");
      if (!name.textContent.startsWith("next · ")) out.push("its name reads '" + name.textContent + "'");
      if (look("v-mode-send") !== sending)
        out.push("the next rung is drawn '" + look("v-mode-send") + "', the sending rung '" + sending + "'");
      const style = getComputedStyle(document.body);
      const probe = document.createElement("span");
      document.body.append(probe);
      const colours = ["--status-tx", "--status-rx", "--status-busy", "--status-warning", "--status-error", "--status-link"]
        .map((t) => style.getPropertyValue(t).trim()).filter(Boolean).map((v) => {
          probe.style.color = v;
          return getComputedStyle(probe).color;
        });
      probe.remove();
      for (const id of ["v-mode-send", "v-mode-send-name"]) {
        if (colours.includes(getComputedStyle($(id)).color)) out.push(id + " is drawn in a state colour");
      }
      window.__screens.metrics({link: null});
      await new Promise((done) => setTimeout(done, 300));
      if (cell.textContent.trim() !== "—") out.push("after the session, Sending reads '" + cell.textContent + "'");
      if (name.textContent) out.push("after the session, its name reads '" + name.textContent + "'");
      return out;
    })()`,
  },
  {
    scene: "answering",
    what: "the other station only answering: Receiving says Control and the family, no dash, no line",
    page: `(() => {
      const out = [], $ = (id) => document.getElementById(id);
      if (!/^\\d+$/.test($("v-mode-send").textContent.trim()))
        out.push("Sending reads '" + $("v-mode-send").textContent + "'");
      if ($("v-mode-recv").textContent.trim() !== "Control")
        out.push("Receiving reads '" + $("v-mode-recv").textContent + "', not Control");
      if ($("v-mode-recv-name").textContent.trim() !== "tone floor")
        out.push("Receiving's family reads '" + $("v-mode-recv-name").textContent + "'");
      if (!$("mode-tile").title.includes("control frames only"))
        out.push("the tile's tooltip does not say control frames only");
      return out;
    })()`,
  },
  {
    scene: "banners",
    what: "idle after a held DISCONNECT: the banner with its two buttons, and no next rung outside a session",
    page: `(() => {
      const out = [], $ = (id) => document.getElementById(id);
      const held = $("held-banner");
      if (held.hidden) out.push("the held banner is hidden");
      if (!held.textContent.includes("inactivity timeout")) out.push("the held banner reads '" + held.textContent.trim() + "'");
      for (const [id, word] of [["btn-held-disconnect", "Disconnect"], ["btn-held-abort", "Abort"]]) {
        const button = $(id);
        if (!button || button.textContent.trim() !== word) out.push("no " + word + " button in it");
      }
      if ($("v-mode-send").textContent.trim() !== "—")
        out.push("idle, Sending reads '" + $("v-mode-send").textContent + "'");
      return out;
    })()`,
  },
  {
    scene: "compact-held",
    // a window shorter than its content, as an operator's own Compact can be once a banner
    // comes: the header kept whole, the banner under it
    define: {held: true, storage: {"aether.view": "compact"}, view: "compact"},
    squeeze: 200,
    what: "Compact during a held DISCONNECT: the banner shows, its × hides it, a new one shows again",
    page: HELD_COMPACT,
  },
  {
    scene: "mini-held",
    define: {held: true, storage: {"aether.view": "mini"}, view: "mini"},
    what: "Mini during a held DISCONNECT: the banner shows over the readings, the header whole",
    page: HELD_COMPACT,
  },
  {
    // a scene of the check's own, not a screenshot (ADR-0137)
    scene: "sound card in another port",
    define: {timeline: "idle", audioCard: {
      fault: null, description: "in Microphone (2- USB Audio CODEC) (1 ch), out Speakers (2- USB Audio CODEC) (2 ch), 48000 Hz",
      input: "Microphone (2- USB Audio CODEC)", output: "Speakers (2- USB Audio CODEC)", tries: 0,
      moved: [
        {direction: "input", configured: "Microphone (USB Audio CODEC)", found: "Microphone (2- USB Audio CODEC)"},
        {direction: "output", configured: "Speakers (USB Audio CODEC)", found: "Speakers (2- USB Audio CODEC)"},
      ]}},
    what: "a codec found under Windows' new number: the banner names it, with Keep",
    page: `(() => {
      const out = [], $ = (id) => document.getElementById(id);
      const banner = $("audio-moved-banner");
      if (banner.hidden) out.push("the banner is hidden");
      const text = $("audio-moved-text").textContent;
      if (!text.includes("Speakers (2- USB Audio CODEC) in place of Speakers (USB Audio CODEC)"))
        out.push("the banner reads '" + text + "'");
      if ($("btn-audio-keep").textContent.trim() !== "Keep") out.push("no Keep button");
      if (!$("btn-audio-moved-dismiss")) out.push("no × to put it away");
      if (!$("btn-audio-keep").title) out.push("Keep has no tooltip");
      if (!$("fault-banner").hidden) out.push("a fault is shown with the card open");
      return out;
    })()`,
  },
  {
    scene: "sound card gone",
    define: {timeline: "idle",
      audioFault: "the playback device went away (The requested device is no longer available)",
      audioCard: {fault: "the playback device went away", description: "", input: null,
                  output: null, moved: [], tries: 3}},
    what: "no sound card: the fault banner says so, its tooltip says what to do, no moved banner",
    page: `(() => {
      const out = [], $ = (id) => document.getElementById(id);
      const fault = $("fault-banner");
      if (fault.hidden) out.push("the fault banner is hidden");
      if (!fault.textContent.startsWith("Sound card unavailable"))
        out.push("the fault banner reads '" + fault.textContent + "'");
      if (!fault.title.includes("48000 Hz")) out.push("its tooltip does not name Windows' format");
      if (!$("audio-moved-banner").hidden) out.push("the moved banner shows with no card");
      return out;
    })()`,
  },
];

async function main() {
  const server = await serve();
  const origin = `http://127.0.0.1:${server.address().port}`;
  const {chrome, profile, url} = await launch();
  const cdp = new Cdp(url);
  await cdp.open();
  let failed = 0;
  try {
    for (const check of CHECKS) {
      // a check's own scene, where the manual has no picture of it
      const scene = check.define ?? {...SCENES[check.scene], ...check.with};
      const opened = await openScene(cdp, origin, check.scene, scene);
      if (check.squeeze) {
        await opened.send("Emulation.setDeviceMetricsOverride", {width: 640,
          height: check.squeeze, deviceScaleFactor: 1, mobile: false});
      }
      await sleep(scene.settle ?? 1500);
      const found = await opened.evaluate(check.page);
      const errors = await opened.evaluate("window.__screens.errors");
      await opened.close();
      const problems = [...found, ...errors, ...opened.problems];
      if (problems.length) failed += 1;
      console.log(`${problems.length ? "FAIL" : "ok  "} ${check.scene}: ${check.what}`);
      for (const problem of problems) console.log(`       ${problem}`);
    }
  } finally {
    chrome.kill();
    server.close();
    await sleep(500);
    await rm(profile, {recursive: true, force: true}).catch(() => {});
  }
  if (failed) process.exit(1);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
