// Aether HF — the panel's colours: dark, or light for a screen in daylight or sunlight.
//
// A plain script, loaded in the head of the panel and of the undocked signal window before
// anything is drawn, so a light panel never opens dark first. The choice is the operator's
// alone (never the operating system's) and is kept in this browser's store, which the panel
// and its undocked window share: one changes, the other follows. Under the desktop shell
// the window's title bar is asked to match.

(() => {
  const KEY = "aether.theme";
  const THEMES = ["dark", "light"];

  function stored() {
    try {
      const value = localStorage.getItem(KEY);
      return THEMES.includes(value) ? value : "dark";
    } catch {
      // a browser that keeps nothing opens dark
      return "dark";
    }
  }

  function titleBar(theme) {
    // the shell's windows open dark; an older shell has no leave to change it, and says so
    // by refusing, which is nothing to report
    try {
      const win = window.__TAURI__?.window?.getCurrentWindow?.();
      win?.setTheme?.(theme)?.catch?.(() => {});
    } catch {
      // a browser, or an older shell
    }
  }

  function apply(theme) {
    if (theme === "light") document.documentElement.dataset.theme = "light";
    else delete document.documentElement.dataset.theme;
    titleBar(theme);
    // the canvases read their colours when they draw; a page that draws on events redraws now
    window.dispatchEvent(new CustomEvent("aether-theme", { detail: theme }));
  }

  /// Change the colours and keep the choice for the next start and the other window.
  function set(theme) {
    if (!THEMES.includes(theme)) theme = "dark";
    try {
      localStorage.setItem(KEY, theme);
    } catch {
      // kept for this page only
    }
    apply(theme);
  }

  window.addEventListener("storage", (event) => {
    if (event.key === KEY) apply(stored());
  });

  window.aetherTheme = {
    get: () => (document.documentElement.dataset.theme === "light" ? "light" : "dark"),
    set,
  };
  apply(stored());
})();
