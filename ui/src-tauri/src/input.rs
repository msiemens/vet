//! What the system settings say about the mouse.

/// How long a second click may follow the first and still make a double click.
#[cfg(not(windows))]
const DEFAULT_DOUBLE_CLICK_MS: u32 = 500;

/// The system's double-click time in milliseconds: the interval of the user's mouse settings.
#[tauri::command]
pub fn double_click_ms() -> u32 {
    system_double_click_ms()
}

#[cfg(target_os = "macos")]
fn system_double_click_ms() -> u32 {
    let seconds = objc2_app_kit::NSEvent::doubleClickInterval();
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1000.0).round() as u32
    } else {
        DEFAULT_DOUBLE_CLICK_MS
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn system_double_click_ms() -> u32 {
    // SAFETY: the call takes no arguments and reads a system setting.
    unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn system_double_click_ms() -> u32 {
    DEFAULT_DOUBLE_CLICK_MS
}
