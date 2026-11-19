// Where the panel's windows go when the panel sizes them.
//
// The shell keeps where a window is between launches, and the panel sets its size, one a view
// (`app.js` for the main window, `signal.js` for the undocked card). A small view parked by the
// screen's right edge grew to Full's size from where it sat and ran half off the screen
// (KK4ODA, 2026-11-19: the header cut at "Idl", the view buttons out of sight). So a window the
// panel sizes is kept inside the work area of the monitor it is on (the screen less the
// taskbar): moved left or up as far as it needs, and made smaller only when it cannot fit at
// all. Plain module, no build step (ADR-0005); `app/tests/placement.test.mjs` holds the sums.

/**
 * Where a window goes, and its inner size, once it is to have the inner size `wanted`. All in
 * physical pixels, as the shell reports them.
 *
 * - `position`: the window's outer top-left now; `outer` and `inner` its outer and inner sizes
 *   now (their difference is the frame, title bar and menu included, which does not change).
 * - `wanted`: the inner size asked for; `least`: the inner size it may not go under.
 * - `area`: the monitor's work area, `{x, y, width, height}`.
 * - `border`: how far the outer rectangle may stand outside the area on each side, for frames
 *   whose outer rectangle includes borders nobody sees (Windows': `windowsBorder`).
 *
 * The size is `wanted` unless the area cannot hold it, then the most it can, never under
 * `least`. The window stays where it is unless it would run past the area's right or bottom
 * edge (it moves left or up) or starts before its left or top edge (it moves right or down).
 * When even `least` is too large the top-left wins: the title bar stays where it can be taken.
 */
export function placeWindow({ position, outer, inner, wanted, least, area, border }) {
  const edge = { left: 0, right: 0, top: 0, bottom: 0, ...border };
  const frameWidth = outer.width - inner.width;
  const frameHeight = outer.height - inner.height;
  const roomWidth = area.width + edge.left + edge.right - frameWidth;
  const roomHeight = area.height + edge.top + edge.bottom - frameHeight;
  const width = Math.max(least.width, Math.min(wanted.width, roomWidth));
  const height = Math.max(least.height, Math.min(wanted.height, roomHeight));
  const right = area.x + area.width + edge.right;
  const bottom = area.y + area.height + edge.bottom;
  let x = position.x;
  let y = position.y;
  if (x + width + frameWidth > right) x = right - width - frameWidth;
  if (x < area.x - edge.left) x = area.x - edge.left;
  if (y + height + frameHeight > bottom) y = bottom - height - frameHeight;
  if (y < area.y - edge.top) y = area.y - edge.top;
  return { x, y, width, height };
}

/**
 * The borders of a Windows window that its outer rectangle counts and nobody sees: since
 * Windows 10 a window's resize borders on the left, right and bottom are invisible, all but
 * the one-pixel line drawn at the frame's edge, and the top has none. A window snapped to the
 * screen's left edge stands that far outside it, and must not be moved for it.
 */
export function windowsBorder(outer, inner) {
  const side = Math.max(0, Math.floor((outer.width - inner.width) / 2) - 1);
  return { left: side, right: side, top: 0, bottom: side };
}

const onWindows = () => /Windows/.test(globalThis.navigator?.userAgent ?? "");

/**
 * Gives a shell window the inner size `size` (logical pixels) and keeps it on its monitor's
 * work area; `size` null keeps the window's size and only brings it back on the screen.
 * Returns the logical size the window has after it. The monitor is the one the window is
 * mostly on before it changes (the shell asks the system for it), so a window by the line
 * between two screens stays on the one it was on.
 *
 * A maximized window is sized as before (on Windows a size restores it), and so is any window
 * of a shell that does not let the panel read its monitor and place its window (one older than
 * this module): the size alone, where it stands.
 */
export async function sizeOnScreen(win, size, least = { width: 0, height: 0 }) {
  const tauri = globalThis.window?.__TAURI__;
  const dpi = tauri?.dpi ?? tauri?.window ?? {};
  const { LogicalSize, PhysicalSize, PhysicalPosition } = dpi;
  let now = null;
  try {
    if (PhysicalSize && PhysicalPosition && !(await win.isMaximized())) {
      const monitor = await tauri.window.currentMonitor();
      if (monitor) {
        const work = monitor.workArea ?? monitor;
        now = {
          area: {
            x: work.position.x,
            y: work.position.y,
            width: work.size.width,
            height: work.size.height,
          },
          position: await win.outerPosition(),
          outer: await win.outerSize(),
          inner: await win.innerSize(),
          scale: await win.scaleFactor(),
        };
      }
    }
  } catch {
    now = null;
  }
  if (!now) {
    if (size) await win.setSize(new LogicalSize(size.width, size.height));
    return size;
  }
  const { area, position, outer, inner, scale } = now;
  const wanted = size
    ? { width: Math.round(size.width * scale), height: Math.round(size.height * scale) }
    : { width: inner.width, height: inner.height };
  const place = placeWindow({
    position,
    outer,
    inner,
    wanted,
    least: { width: Math.round(least.width * scale), height: Math.round(least.height * scale) },
    area,
    border: onWindows() ? windowsBorder(outer, inner) : null,
  });
  const moves = place.x !== position.x || place.y !== position.y;
  const fits = place.width === wanted.width && place.height === wanted.height;
  // the size as asked when it fits, so a window is exactly what it was before this module
  const resize = fits
    ? size && new LogicalSize(size.width, size.height)
    : new PhysicalSize(place.width, place.height);
  // a window that grows moves first and one that shrinks after, so it never stands off the
  // screen between the two
  const grows = place.width > inner.width || place.height > inner.height;
  if (moves && grows) await win.setPosition(new PhysicalPosition(place.x, place.y));
  if (resize) await win.setSize(resize);
  if (moves && !grows) await win.setPosition(new PhysicalPosition(place.x, place.y));
  return fits && size ? size : { width: place.width / scale, height: place.height / scale };
}
