//! The app-drawn title bar and showing the window.

use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_decoration::WebviewWindowExt;

use crate::{geometry, startup};

/// How long one activation of the app-drawn title bar may take before the plugin restores the
/// native frame; a new attempt challenges the page afresh.
pub const DECORATION_TIMEOUT: Duration = Duration::from_secs(4);
const DECORATION_ATTEMPTS: u32 = 3;
/// The window shows by then even while the app-drawn title bar is still activating.
const SHOW_WITHIN: Duration = Duration::from_secs(2);

/// Replaces the native title bar with the app header (window controls drawn by
/// tauri-plugin-decoration, native traffic lights on macOS) and shows the window. If that
/// fails, the native frame is restored and shown instead. Returns "custom" or "native".
#[tauri::command]
pub async fn activate_and_show(window: tauri::WebviewWindow) -> Result<&'static str, String> {
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
///
/// The main window's activation starts with its page, long before the page asks for it in
/// [`activate_and_show`], which joins it. tauri-plugin-decoration 3.0.5's script announces the page
/// as it starts and retries for less than 2 s; an activation in that page completes only if one was
/// pending during the announcement. Without one the plugin answers in a shape the script rejects,
/// and every activation of that page times out.
pub fn on_page_load(webview: &tauri::Webview, payload: &tauri::webview::PageLoadPayload) {
    if payload.event() != tauri::webview::PageLoadEvent::Started {
        return;
    }
    #[cfg(target_os = "macos")]
    if let Err(e) = webview.window().set_title_bar_style(tauri::TitleBarStyle::Visible) {
        tracing::warn!(error = %e, "cannot give the window its native title bar");
    }
    if webview.label() != geometry::MAIN {
        return;
    }
    let Some(window) = webview.app_handle().get_webview_window(webview.label()) else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        if let Err(e) = window.activate_decoration().await {
            tracing::debug!(error = %e, "app-drawn title bar not active as the page started");
        }
    });
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
