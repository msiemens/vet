//! Letting the page answer a request to end the app. While a comment box of the page holds text that
//! was not added, the page arms the guard; a close or exit request that reaches the host then goes to
//! the page, which asks the viewer and ends the app with `quit` if the text may be discarded. With
//! the guard disarmed every request is the host's own, so a page that hangs or reloads never holds
//! the window shut.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, Manager, State, Window, WindowEvent};

use crate::geometry::MAIN;

/// Sent to the main window when a request to end the app is held for the page's answer.
const EVENT: &str = "vet-close-requested";

/// Whether the page wants to answer requests to end the app.
#[derive(Default)]
pub struct CloseGuard(AtomicBool);

/// A request to end the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// The main window was asked to close: its close button, Alt+F4, the window menu.
    Window,
    /// The app was asked to exit with no code: the last window closed, Cmd+Q, the Dock's Quit.
    Exit,
    /// The app was told to exit with a code, as `quit` does after the page agreed.
    ExitWith,
}

/// What the host does with a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// It goes ahead.
    Let,
    /// It is held and the page is asked.
    Ask,
}

impl CloseGuard {
    fn set(&self, armed: bool) {
        self.0.store(armed, Ordering::SeqCst);
    }

    fn armed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// What to do with `request`: only a request the page has not seen is held, and only while armed.
    pub fn answer(&self, request: Request) -> Answer {
        match request {
            Request::ExitWith => Answer::Let,
            Request::Window | Request::Exit if self.armed() => Answer::Ask,
            Request::Window | Request::Exit => Answer::Let,
        }
    }

    /// The page loads afresh and knows nothing of what it armed.
    pub fn disarm(&self) {
        self.set(false);
    }
}

/// The page arms the guard while a comment box holds text, and disarms it when none does.
#[tauri::command]
pub fn guard_close(guard: State<CloseGuard>, armed: bool) {
    guard.set(armed);
}

fn ask_page(app: &AppHandle) {
    if let Err(e) = app.emit_to(MAIN, EVENT, ()) {
        tracing::warn!(error = %e, "cannot ask the page about the close request");
    }
}

/// Holds a close of the main window for the page when the guard is armed.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() != MAIN {
        return;
    }
    let Some(guard) = window.try_state::<CloseGuard>() else {
        return;
    };
    if guard.answer(Request::Window) == Answer::Ask {
        api.prevent_close();
        ask_page(window.app_handle());
    }
}

/// Holds an exit the app was not told to make for the page when the guard is armed.
pub fn on_exit_requested(app: &AppHandle, code: Option<i32>, api: &tauri::ExitRequestApi) {
    let Some(guard) = app.try_state::<CloseGuard>() else {
        return;
    };
    let request = if code.is_some() {
        Request::ExitWith
    } else {
        Request::Exit
    };
    if guard.answer(request) == Answer::Ask {
        api.prevent_exit();
        ask_page(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disarmed_guard_lets_every_request_go() {
        let guard = CloseGuard::default();
        for request in [Request::Window, Request::Exit, Request::ExitWith] {
            assert_eq!(guard.answer(request), Answer::Let, "{request:?}");
        }
    }

    #[test]
    fn an_armed_guard_holds_what_the_page_has_not_seen_for_it() {
        let guard = CloseGuard::default();
        guard.set(true);
        assert_eq!(guard.answer(Request::Window), Answer::Ask);
        assert_eq!(guard.answer(Request::Exit), Answer::Ask);
    }

    #[test]
    fn the_exit_the_page_asked_for_is_never_held() {
        let guard = CloseGuard::default();
        guard.set(true);
        assert_eq!(guard.answer(Request::ExitWith), Answer::Let);
    }

    #[test]
    fn a_page_that_loads_afresh_disarms_the_guard() {
        let guard = CloseGuard::default();
        guard.set(true);
        guard.disarm();
        assert_eq!(guard.answer(Request::Window), Answer::Let);
    }
}
