// Where the panel puts its windows: `app/ui/placement.js`. Run with
// `node --test app/tests/placement.test.mjs`.
//
// The panel's scripts have no package.json saying they are ES modules (the daemon serves the
// directory as it stands, ADR-0005), so the module is read and imported from its text.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../ui/placement.js", import.meta.url), "utf8");
const { placeWindow, windowsBorder, sizeOnScreen } = await import(
  `data:text/javascript,${encodeURIComponent(source)}`
);

// A 1920 × 1080 screen at 100 % with a 48-pixel taskbar along the bottom, and a Windows frame:
// 8 pixels each side and at the bottom (7 of them invisible), 31 at the top with the title bar.
const AREA = { x: 0, y: 0, width: 1920, height: 1032 };
const FRAME = { width: 16, height: 39 };
const BORDER = { left: 7, right: 7, top: 0, bottom: 7 };
const LEAST = { width: 420, height: 300 };

function windowAt(x, y, width, height) {
  return {
    position: { x, y },
    inner: { width, height },
    outer: { width: width + FRAME.width, height: height + FRAME.height },
  };
}

test("a window that fits stays where it is, at the size asked", () => {
  const place = placeWindow({
    ...windowAt(300, 200, 640, 300),
    wanted: { width: 1000, height: 700 },
    least: LEAST,
    area: AREA,
    border: BORDER,
  });
  assert.deepEqual(place, { x: 300, y: 200, width: 1000, height: 700 });
});

test("Compact parked by the right edge grows to Full leftwards, its right edge at the screen's", () => {
  // the field case of 2026-11-19: Compact at x 1200, Full 1200 wide
  const place = placeWindow({
    ...windowAt(1200, 100, 640, 300),
    wanted: { width: 1200, height: 800 },
    least: LEAST,
    area: AREA,
    border: BORDER,
  });
  assert.deepEqual(place, { x: 1920 + 7 - 1216, y: 100, width: 1200, height: 800 });
  // the visible frame ends at the screen's last column, the invisible border beyond it
  assert.equal(place.x + place.width + FRAME.width - BORDER.right, 1920);
});

test("a window by the taskbar grows upwards, its bottom on the taskbar's top", () => {
  const place = placeWindow({
    ...windowAt(100, 800, 640, 200),
    wanted: { width: 1000, height: 700 },
    least: LEAST,
    area: AREA,
    border: BORDER,
  });
  assert.equal(place.y + 700 + FRAME.height - BORDER.bottom, 1032);
  assert.equal(place.x, 100);
});

test("a size the screen cannot hold is made as large as the work area, and no larger", () => {
  const place = placeWindow({
    ...windowAt(500, 400, 640, 300),
    wanted: { width: 2400, height: 1400 },
    least: LEAST,
    area: AREA,
    border: BORDER,
  });
  assert.deepEqual(place, {
    x: -7,
    y: 0,
    width: 1920 + 14 - FRAME.width,
    height: 1032 + 7 - FRAME.height,
  });
});

test("when even the least size does not fit, the title bar stays on the screen", () => {
  const place = placeWindow({
    ...windowAt(100, 100, 640, 300),
    wanted: { width: 1000, height: 700 },
    least: { width: 900, height: 500 },
    area: { x: 0, y: 0, width: 800, height: 480 },
    border: null,
  });
  assert.deepEqual(place, { x: 0, y: 0, width: 900, height: 500 });
});

test("a window snapped to the left edge is not moved for its invisible border", () => {
  const place = placeWindow({
    ...windowAt(-7, 0, 640, 300),
    wanted: { width: 1200, height: 800 },
    least: LEAST,
    area: AREA,
    border: BORDER,
  });
  assert.deepEqual(place, { x: -7, y: 0, width: 1200, height: 800 });
});

test("a window left over from a screen no longer there comes back onto the one it is nearest", () => {
  // a second monitor left of the main one, 2560 × 1400 at 150 %, its taskbar 72 high
  const area = { x: -2560, y: 0, width: 2560, height: 1368 };
  const place = placeWindow({
    position: { x: -2900, y: -50 },
    inner: { width: 1500, height: 1050 },
    outer: { width: 1524, height: 1108 },
    wanted: { width: 1500, height: 1050 },
    least: { width: 630, height: 450 },
    area,
    border: { left: 11, right: 11, top: 0, bottom: 11 },
  });
  assert.deepEqual(place, { x: -2560 - 11, y: 0, width: 1500, height: 1050 });
});

test("a window on a screen to the right is kept on that screen", () => {
  const area = { x: 1920, y: 0, width: 1920, height: 1040 };
  const place = placeWindow({
    ...windowAt(3500, 300, 520, 200),
    wanted: { width: 1100, height: 760 },
    least: LEAST,
    area,
    border: BORDER,
  });
  assert.equal(place.x + 1100 + FRAME.width - BORDER.right, 3840);
  assert.ok(place.x > 1920);
});

test("off Windows, no border: the frame's edge is the window's", () => {
  const place = placeWindow({
    position: { x: 1500, y: 0 },
    inner: { width: 640, height: 300 },
    outer: { width: 640, height: 328 },
    wanted: { width: 1000, height: 700 },
    least: LEAST,
    area: { x: 0, y: 25, width: 1440, height: 875 },
    border: null,
  });
  assert.deepEqual(place, { x: 440, y: 25, width: 1000, height: 700 });
});

test("Windows' invisible borders, read from the frame", () => {
  assert.deepEqual(windowsBorder({ width: 1016, height: 739 }, { width: 1000, height: 700 }), {
    left: 7,
    right: 7,
    top: 0,
    bottom: 7,
  });
  assert.deepEqual(windowsBorder({ width: 1523, height: 1100 }, { width: 1500, height: 1046 }), {
    left: 10,
    right: 10,
    top: 0,
    bottom: 10,
  });
  assert.deepEqual(windowsBorder({ width: 640, height: 328 }, { width: 640, height: 300 }), {
    left: 0,
    right: 0,
    top: 0,
    bottom: 0,
  });
});

// ── the shell's calls, against a stand-in window ─────────────────────

class LogicalSize {
  constructor(width, height) {
    Object.assign(this, { type: "Logical", width, height });
  }
}
class PhysicalSize {
  constructor(width, height) {
    Object.assign(this, { type: "Physical", width, height });
  }
}
class PhysicalPosition {
  constructor(x, y) {
    Object.assign(this, { type: "Physical", x, y });
  }
}

/// A shell window at `x, y` with inner size `width × height` (physical), on `area`; `refuses`
/// stands for an older shell, whose panel may not read its monitor or place its window.
function shell({ x, y, width, height, scale = 1, maximized = false, refuses = false, area = AREA }) {
  const calls = [];
  const win = {
    async isMaximized() {
      return maximized;
    },
    async outerPosition() {
      if (refuses) throw new Error("window.outer_position not allowed");
      return { x, y };
    },
    async outerSize() {
      return { width: width + FRAME.width, height: height + FRAME.height };
    },
    async innerSize() {
      return { width, height };
    },
    async scaleFactor() {
      return scale;
    },
    async setSize(size) {
      calls.push(["size", size.type, size.width, size.height]);
    },
    async setPosition(position) {
      calls.push(["position", position.x, position.y]);
    },
  };
  globalThis.window = {
    __TAURI__: {
      dpi: { LogicalSize, PhysicalSize, PhysicalPosition },
      window: {
        async currentMonitor() {
          if (refuses) throw new Error("window.current_monitor not allowed");
          return {
            scaleFactor: scale,
            position: { x: area.x, y: area.y },
            size: { width: area.width, height: area.height + 48 },
            workArea: {
              position: { x: area.x, y: area.y },
              size: { width: area.width, height: area.height },
            },
          };
        },
      },
    },
  };
  Object.defineProperty(globalThis, "navigator", {
    value: { userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/140.0" },
    configurable: true,
  });
  return { win, calls };
}

test("growing by the right edge, the window moves before it grows", async () => {
  const { win, calls } = shell({ x: 1200, y: 100, width: 640, height: 300 });
  const set = await sizeOnScreen(win, { width: 1200, height: 800 }, LEAST);
  assert.deepEqual(calls, [
    ["position", 711, 100],
    ["size", "Logical", 1200, 800],
  ]);
  assert.deepEqual(set, { width: 1200, height: 800 });
});

test("a window that fits is only sized, exactly as asked", async () => {
  const { win, calls } = shell({ x: 200, y: 100, width: 1200, height: 800, scale: 1.25 });
  const set = await sizeOnScreen(win, { width: 640, height: 260 }, { width: 360, height: 120 });
  assert.deepEqual(calls, [["size", "Logical", 640, 260]]);
  assert.deepEqual(set, { width: 640, height: 260 });
});

test("too large for the screen at 125 %, it is sized in physical pixels and says so", async () => {
  const { win, calls } = shell({ x: 300, y: 300, width: 800, height: 375, scale: 1.25 });
  const set = await sizeOnScreen(win, { width: 1600, height: 1000 }, LEAST);
  // 1920 + 14 − 16 = 1918 wide, 1032 + 7 − 39 = 1000 high, at the top-left
  assert.deepEqual(calls, [
    ["position", -7, 0],
    ["size", "Physical", 1918, 1000],
  ]);
  assert.deepEqual(set, { width: 1918 / 1.25, height: 1000 / 1.25 });
});

test("with no size, a window off the screen is brought back and not resized", async () => {
  const { win, calls } = shell({ x: 1700, y: 900, width: 1000, height: 700 });
  const set = await sizeOnScreen(win, null, LEAST);
  assert.deepEqual(calls, [["position", 1920 + 7 - 1016, 1032 + 7 - 739]]);
  assert.deepEqual(set, { width: 1000, height: 700 });
});

test("with no size, a window on the screen is left alone", async () => {
  const { win, calls } = shell({ x: 100, y: 100, width: 1000, height: 700 });
  await sizeOnScreen(win, null, LEAST);
  assert.deepEqual(calls, []);
});

test("shrinking, the window is sized before it moves", async () => {
  // a small view of a window that stood half off the screen's right edge
  const { win, calls } = shell({ x: 1500, y: 100, width: 1200, height: 800 });
  await sizeOnScreen(win, { width: 640, height: 300 }, { width: 360, height: 120 });
  assert.deepEqual(calls, [
    ["size", "Logical", 640, 300],
    ["position", 1920 + 7 - 656, 100],
  ]);
});

test("an older shell's panel sizes its window as before", async () => {
  const { win, calls } = shell({ x: 1200, y: 100, width: 640, height: 300, refuses: true });
  const set = await sizeOnScreen(win, { width: 1200, height: 800 }, LEAST);
  assert.deepEqual(calls, [["size", "Logical", 1200, 800]]);
  assert.deepEqual(set, { width: 1200, height: 800 });
});

test("a maximized window is sized as before, never moved", async () => {
  const { win, calls } = shell({ x: -8, y: -8, width: 1920, height: 1001, maximized: true });
  await sizeOnScreen(win, { width: 640, height: 300 }, { width: 360, height: 120 });
  assert.deepEqual(calls, [["size", "Logical", 640, 300]]);
});
