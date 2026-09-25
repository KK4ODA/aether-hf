//! The signal-analysis card, undocked.
//!
//! The panel's constellation, spectrum and waterfall can leave the Diagnostics tab for a
//! window of their own (`app/ui/signal.html`, served by the daemon like the panel), so they
//! stay in view whatever tab the panel shows. The panel asks through events — it is served by
//! the daemon and has no commands (`capabilities/panel.json`) — and the shell owns the window:
//! it opens one and only one, brings it forward when asked again, closes it when the card is
//! docked, and tells the panel whenever it opens or goes, so the card is drawn in one place
//! at a time.

use tauri::{AppHandle, Emitter as _, Manager as _};

/// The window's label, which the panel's capability names.
pub const WINDOW: &str = "signal";

/// What the panel hears about the window: whether it is open.
const EVENT: &str = "signal-window";

/// Open the window, or bring it to the front if it is already open.
pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.unminimize();
        let _ = window.set_focus();
        tell_panel(app, true);
        return;
    }
    let Ok(url) = format!("http://{}/signal.html", crate::CONTROL).parse::<tauri::Url>() else {
        return;
    };
    let built = tauri::WebviewWindowBuilder::new(app, WINDOW, tauri::WebviewUrl::External(url))
        .title("Aether HF — signal analysis")
        .inner_size(860.0, 640.0)
        .min_inner_size(420.0, 320.0)
        .resizable(true)
        .theme(Some(tauri::Theme::Dark))
        .build();
    // a window that could not be made leaves the card where it was, and the panel says so
    tell_panel(app, built.is_ok());
}

/// Close the window: the card goes back to the panel. The window's own `Destroyed` tells
/// the panel; with no window there is nothing to wait for.
pub fn close(app: &AppHandle) {
    match app.get_webview_window(WINDOW) {
        Some(window) => {
            if window.close().is_err() {
                let _ = window.destroy();
            }
        }
        None => tell_panel(app, false),
    }
}

/// Tell the panel where the card is: in its own window, or back on the Diagnostics tab.
pub fn tell_panel(app: &AppHandle, open: bool) {
    let _ = app.emit_to("main", EVENT, serde_json::json!({ "open": open }));
}

/// Whether the window is open now: what the panel asks for when it (re)loads.
pub fn is_open(app: &AppHandle) -> bool {
    app.get_webview_window(WINDOW).is_some()
}
