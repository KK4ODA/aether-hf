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
    scene: "banners",
    what: "idle after a held DISCONNECT: the banner with its two buttons, and the next rung",
    page: `(() => {
      const out = [], $ = (id) => document.getElementById(id);
      const held = $("held-banner");
      if (held.hidden) out.push("the held banner is hidden");
      if (!held.textContent.includes("inactivity timeout")) out.push("the held banner reads '" + held.textContent.trim() + "'");
      for (const [id, word] of [["btn-held-disconnect", "Disconnect"], ["btn-held-abort", "Abort"]]) {
        const button = $(id);
        if (!button || button.textContent.trim() !== word) out.push("no " + word + " button in it");
      }
      if (!/^next: \\d+$/.test($("v-mode-send").textContent.trim()))
        out.push("idle, Sending reads '" + $("v-mode-send").textContent + "'");
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
      const scene = SCENES[check.scene];
      const opened = await openScene(cdp, origin, check.scene, scene);
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
