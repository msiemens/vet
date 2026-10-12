//! The app-drawn title bar and showing the window.

use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::startup;

/// How long one activation of the app-drawn title bar may take before the plugin restores the
/// native frame. Now and then its page handshake gets lost and an attempt only ends here; a new
/// attempt challenges the then-loaded page afresh.
pub const DECORATION_TIMEOUT: Duration = Duration::from_secs(4);
const DECORATION_ATTEMPTS: u32 = 3;
/// The window shows by then even while the app-drawn title bar is still activating.
const SHOW_WITHIN: Duration = Duration::from_secs(2);

/// Replaces the native title bar with the app header (window controls drawn by
/// tauri-plugin-decoration, native traffic lights on macOS) and shows the window. If that
/// fails, the native frame is restored and shown instead. Returns "custom" or "native".
#[tauri::command]
pub async fn activate_and_show(window: tauri::WebviewWindow) -> Result<&'static str, String> {
    use tauri_plugin_decoration::WebviewWindowExt;

    async fn native(window: &tauri::WebviewWindow, cause: impl std::fmt::Display) -> Result<&'static str, String> {
        tracing::warn!(%cause, "app-drawn title bar unavailable, using the native frame");
        let restored = window.restore_decoration().await;
        #[cfg(target_os = "macos")]
        if let Err(e) = window.set_title_bar_style(tauri::TitleBarStyle::Visible) {
            tracing::warn!(error = %e, "cannot give the native frame its own title bar");
        }
        reveal(window).map_err(|e| format!("{cause}; showing the native frame failed: {e}"))?;
        restored
            .map(|()| "native")
            .map_err(|e| format!("{cause}; restoring the native frame failed: {e}"))
    }

    // The window shows while activation is still pending; the app-drawn title bar replaces the
    // frame once it comes.
    let shown = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_WITHIN);
        if let Err(e) = reveal(&shown) {
            tracing::warn!(error = %e, "cannot show the window");
        }
    });
    let mut attempt = 1;
    while let Err(e) = window.activate_decoration().await {
        if attempt == DECORATION_ATTEMPTS {
            return native(&window, e).await;
        }
        tracing::warn!(attempt, of = DECORATION_ATTEMPTS, error = %e, "app-drawn title bar failed");
        attempt += 1;
    }
    // The app header runs under the traffic lights, which only the activated title bar makes room
    // for; until then the native title bar keeps the page below them.
    #[cfg(target_os = "macos")]
    if let Err(e) = window.set_title_bar_style(tauri::TitleBarStyle::Overlay) {
        return native(&window, e).await;
    }
    // Centres the traffic lights in the 40px header: the plugin centres them in a titlebar of
    // their own height (14pt) plus `y`, so `y` is what makes that titlebar 40pt tall.
    #[cfg(target_os = "macos")]
    if let Err(e) = window.set_traffic_lights_inset(16.0, 26.0).await {
        return native(&window, e).await;
    }
    #[cfg(debug_assertions)]
    if std::env::var_os("VET_DEVTOOLS").is_some() {
        window.open_devtools();
    }
    match reveal(&window) {
        Ok(()) => Ok("custom"),
        Err(e) => native(&window, e).await,
    }
}

/// A page that starts loading has no app-drawn title bar yet (the plugin drops it with the old
/// page), so on macOS the window gets its native title bar back until the page activates one.
pub fn on_page_load(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload) {
    #[cfg(target_os = "macos")]
    if payload.event() == tauri::webview::PageLoadEvent::Started
        && let Err(e) = webview.window().set_title_bar_style(tauri::TitleBarStyle::Visible)
    {
        tracing::warn!(error = %e, "cannot give the window its native title bar");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (webview, payload);
}

/// Shows the main window, then closes the splash that covered its start.
fn reveal(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    window.show()?;
    startup::close_splash(window.app_handle());
    Ok(())
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}
