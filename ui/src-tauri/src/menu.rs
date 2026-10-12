//! The native menu bar and context menus, which the UI defines.

use std::collections::HashSet;
use std::sync::Mutex;
use std::sync::mpsc;

use tauri::menu::{
    CheckMenuItemBuilder, ContextMenu, IsMenuItem, Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, Submenu,
};
use tauri::{AppHandle, LogicalPosition, Manager, State, Window, Wry};

/// An item the OS defines itself: its label, key and behaviour are the system's (`PredefinedItem`
/// in `ui/src/lib/ui/menuItem.ts`).
#[derive(serde::Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
enum Predefined {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Minimize,
    Zoom,
    Fullscreen,
    BringAllToFront,
}

impl Predefined {
    fn item(self, app: &AppHandle) -> tauri::Result<PredefinedMenuItem<Wry>> {
        match self {
            Self::Undo => PredefinedMenuItem::undo(app, None),
            Self::Redo => PredefinedMenuItem::redo(app, None),
            Self::Cut => PredefinedMenuItem::cut(app, None),
            Self::Copy => PredefinedMenuItem::copy(app, None),
            Self::Paste => PredefinedMenuItem::paste(app, None),
            Self::SelectAll => PredefinedMenuItem::select_all(app, None),
            Self::Services => PredefinedMenuItem::services(app, None),
            Self::Hide => PredefinedMenuItem::hide(app, None),
            Self::HideOthers => PredefinedMenuItem::hide_others(app, None),
            Self::ShowAll => PredefinedMenuItem::show_all(app, None),
            Self::Minimize => PredefinedMenuItem::minimize(app, None),
            Self::Zoom => PredefinedMenuItem::maximize(app, None),
            Self::Fullscreen => PredefinedMenuItem::fullscreen(app, None),
            Self::BringAllToFront => PredefinedMenuItem::bring_all_to_front(app, None),
        }
    }
}

/// One entry of a menu the UI defines (`MenuItem` in `ui/src/lib/ui/menuItem.ts`).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuEntry {
    id: String,
    label: String,
    /// Secondary text; a native menu has no column for it, so it follows the label.
    detail: Option<String>,
    /// The key that runs the item in the window, as the viewer reads it (`⌃⌥C`); the menu only shows it and the window handles the key.
    shortcut: Option<String>,
    /// The same key as an accelerator reads it (`Ctrl+Alt+C`), for a menu that draws it in the OS's key column (macOS).
    chord: Option<String>,
    /// A key the menu registers and handles itself, such as `Cmd+,`.
    accelerator: Option<String>,
    enabled: Option<bool>,
    checked: Option<bool>,
    #[serde(default)]
    separator: bool,
    predefined: Option<Predefined>,
    children: Option<Vec<MenuEntry>>,
}

/// Where a menu is shown, which decides how its items show their shortcuts.
#[derive(Clone, Copy, PartialEq)]
enum Surface {
    MenuBar,
    ContextMenu,
}

/// The key a menu item draws in the OS's key column: its own accelerator or, on macOS, the
/// window's key (muda reads the chord text `Cmd+B`, `Ctrl+Alt+Down`, `F4`, `Space` as an
/// accelerator). Only an item's own accelerator is the menu's to handle: the window's key is
/// handled by the window, so a menu bar item that draws it ignores the menu event the key raises
/// (`KeyOnlyItems`), and a context menu is not attached to the window, so its accelerators fire
/// only while it is open.
fn accelerator(entry: &MenuEntry) -> Option<&str> {
    if let Some(accelerator) = &entry.accelerator {
        return Some(accelerator);
    }
    if cfg!(target_os = "macos") {
        return entry.chord.as_deref();
    }
    None
}

/// The text of an item: its label, the detail after it, and the display shortcut where the menu
/// has no column for it (`keyed` is false: no accelerator drew it). Windows right-aligns text after
/// a tab as the shortcut column; other menu bars put it after the label, other context menus show none.
fn item_text(entry: &MenuEntry, surface: Surface, keyed: bool) -> String {
    let mut text = entry.label.clone();
    if let Some(detail) = &entry.detail {
        text = format!("{text}  —  {detail}");
    }
    if let (Some(shortcut), false) = (&entry.shortcut, keyed) {
        if cfg!(windows) {
            text = format!("{text}\t{shortcut}");
        } else if surface == Surface::MenuBar {
            text = format!("{text}    {shortcut}");
        }
    }
    text
}

fn item(app: &AppHandle, entry: &MenuEntry, surface: Surface) -> tauri::Result<Box<dyn IsMenuItem<Wry>>> {
    if entry.separator {
        return Ok(Box::new(PredefinedMenuItem::separator(app)?));
    }
    if let Some(predefined) = entry.predefined {
        return Ok(Box::new(predefined.item(app)?));
    }
    let enabled = entry.enabled.unwrap_or(true);
    if let Some(children) = &entry.children {
        return Ok(Box::new(submenu(app, &entry.label, enabled, children, surface)?));
    }
    let accelerator = accelerator(entry);
    // A shortcut that is no accelerator is shown as text rather than failing the menu.
    let build = |accelerator: Option<&str>| -> tauri::Result<Box<dyn IsMenuItem<Wry>>> {
        let text = item_text(entry, surface, accelerator.is_some());
        Ok(if let Some(checked) = entry.checked {
            let mut item = CheckMenuItemBuilder::with_id(&entry.id, &text)
                .checked(checked)
                .enabled(enabled);
            if let Some(accelerator) = accelerator {
                item = item.accelerator(accelerator);
            }
            Box::new(item.build(app)?)
        } else {
            let mut item = MenuItemBuilder::with_id(&entry.id, &text).enabled(enabled);
            if let Some(accelerator) = accelerator {
                item = item.accelerator(accelerator);
            }
            Box::new(item.build(app)?)
        })
    };
    match build(accelerator) {
        Err(_) if entry.accelerator.is_none() && accelerator.is_some() => build(None),
        built => built,
    }
}

fn items(app: &AppHandle, entries: &[MenuEntry], surface: Surface) -> tauri::Result<Vec<Box<dyn IsMenuItem<Wry>>>> {
    entries.iter().map(|entry| item(app, entry, surface)).collect()
}

fn submenu(
    app: &AppHandle,
    label: &str,
    enabled: bool,
    entries: &[MenuEntry],
    surface: Surface,
) -> tauri::Result<Submenu<Wry>> {
    let built = items(app, entries, surface)?;
    let refs: Vec<&dyn IsMenuItem<Wry>> = built.iter().map(|i| i.as_ref()).collect();
    Submenu::with_items(app, label, enabled, &refs)
}

/// The menu id of the Window menu, which the OS lists the open windows in.
#[cfg(target_os = "macos")]
const WINDOW_MENU: &str = "window";

/// The ids of the menu bar items that draw the window's key and have no accelerator of their own.
/// The webview sees a key before the menu does and the window runs its command there, so the
/// menu event such a key raises afterwards (a key the window left alone, such as one typed in a
/// text field) must not run the command a second time.
#[derive(Default)]
pub struct KeyOnlyItems(Mutex<HashSet<String>>);

fn key_only_ids(entries: &[MenuEntry], ids: &mut HashSet<String>) {
    for entry in entries {
        if entry.accelerator.is_none() && accelerator(entry).is_some() {
            ids.insert(entry.id.clone());
        }
        key_only_ids(entry.children.as_deref().unwrap_or_default(), ids);
    }
}

/// Whether the menu event of item `id` was raised by its key: the item draws the window's key and
/// the event being handled is a key press, not a click.
pub fn raised_by_window_key(app: &AppHandle, id: &str) -> bool {
    app.state::<KeyOnlyItems>()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(id)
        && key_press_in_progress()
}

#[cfg(target_os = "macos")]
fn key_press_in_progress() -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEventType};
    MainThreadMarker::new()
        .and_then(|main| NSApplication::sharedApplication(main).currentEvent())
        .is_some_and(|event| event.r#type() == NSEventType::KeyDown)
}

#[cfg(not(target_os = "macos"))]
fn key_press_in_progress() -> bool {
    false
}

/// Replaces the menu bar (macOS) with the UI's definition. The UI includes the Edit menu as
/// predefined items: without it the webview does not get ⌘C, ⌘V and ⌘Z.
#[tauri::command]
pub fn set_menu(app: AppHandle, state: State<'_, KeyOnlyItems>, items: Vec<MenuEntry>) -> Result<(), String> {
    let mut key_only = HashSet::new();
    key_only_ids(&items, &mut key_only);
    *state.0.lock().unwrap_or_else(|e| e.into_inner()) = key_only;
    let build = || -> tauri::Result<()> {
        let mut bar = MenuBuilder::new(&app);
        for menu in &items {
            let children = menu.children.as_deref().unwrap_or_default();
            let built = submenu(
                &app,
                &menu.label,
                menu.enabled.unwrap_or(true),
                children,
                Surface::MenuBar,
            )?;
            #[cfg(target_os = "macos")]
            if menu.id == WINDOW_MENU {
                built.set_as_windows_menu_for_nsapp()?;
            }
            bar = bar.item(&built);
        }
        app.set_menu(bar.build()?)?;
        Ok(())
    };
    build().map_err(|e| e.to_string())
}

/// The menu bar from launch until the UI sends its own: the app menu, Edit and Window with only
/// what the OS defines, so nothing appears that the UI's menu then takes away.
#[cfg(target_os = "macos")]
pub fn startup_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let about = PredefinedMenuItem::about(app, None, None)?;
    let quit = PredefinedMenuItem::quit(app, None)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let vet = Submenu::with_items(app, "Vet", true, &[&about, &separator, &quit])?;
    let block = |label: &str, kinds: &[Option<Predefined>]| -> tauri::Result<Submenu<Wry>> {
        let built = kinds
            .iter()
            .map(|kind| -> tauri::Result<Box<dyn IsMenuItem<Wry>>> {
                Ok(match kind {
                    Some(kind) => Box::new(kind.item(app)?),
                    None => Box::new(PredefinedMenuItem::separator(app)?),
                })
            })
            .collect::<tauri::Result<Vec<_>>>()?;
        let refs: Vec<&dyn IsMenuItem<Wry>> = built.iter().map(|i| i.as_ref()).collect();
        Submenu::with_items(app, label, true, &refs)
    };
    use Predefined::*;
    let edit = block(
        "Edit",
        &[
            Some(Undo),
            Some(Redo),
            None,
            Some(Cut),
            Some(Copy),
            Some(Paste),
            Some(SelectAll),
        ],
    )?;
    let window = block(
        "Window",
        &[
            Some(Minimize),
            Some(Zoom),
            None,
            Some(Fullscreen),
            None,
            Some(BringAllToFront),
        ],
    )?;
    window.set_as_windows_menu_for_nsapp()?;
    Menu::with_items(app, &[&vet, &edit, &window])
}

/// Who waits for the choice in the context menu that is open: the item chosen, or `None` once the
/// menu closed without one.
#[derive(Default)]
pub struct ContextMenuChoice(Mutex<Option<mpsc::Sender<Option<String>>>>);

impl ContextMenuChoice {
    fn begin(&self) -> mpsc::Receiver<Option<String>> {
        let (tx, rx) = mpsc::channel();
        // A menu still pending is abandoned: its receiver sees the channel close.
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        rx
    }

    /// Answers the waiting context menu; `false` if none is open.
    fn answer(&self, choice: Option<String>) -> bool {
        let sender = self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
        sender.is_some_and(|tx| tx.send(choice).is_ok())
    }
}

/// Hands an item chosen in a native menu to the context menu that is open, if there is one.
/// Answers whether it did; otherwise the item belongs to the menu bar.
pub fn answer_context_menu(app: &AppHandle, id: &str) -> bool {
    app.state::<ContextMenuChoice>().answer(Some(id.to_owned()))
}

/// Pops up a context menu at `x`, `y` (CSS pixels from the window's top left) and answers the id of
/// the item chosen, or `None` if the menu closed without one.
#[tauri::command]
pub async fn show_context_menu(
    app: AppHandle,
    window: Window,
    state: State<'_, ContextMenuChoice>,
    items: Vec<MenuEntry>,
    x: f64,
    y: f64,
) -> Result<Option<String>, String> {
    let choice = state.begin();
    let popup = move || -> tauri::Result<()> {
        let built = self::items(&app, &items, Surface::ContextMenu)?;
        let refs: Vec<&dyn IsMenuItem<Wry>> = built.iter().map(|i| i.as_ref()).collect();
        let menu = Menu::with_items(&app, &refs)?;
        // Blocks until the menu closes.
        menu.popup_at(window, LogicalPosition::new(x, y))?;
        // A chosen item reaches the event loop before this task does, so what is still waiting then
        // was dismissed.
        let handle = app.clone();
        app.run_on_main_thread(move || {
            handle.state::<ContextMenuChoice>().answer(None);
        })
    };
    tauri::async_runtime::spawn_blocking(move || {
        popup().map_err(|e| e.to_string())?;
        Ok(choice.recv().unwrap_or(None))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::{ContextMenuChoice, MenuEntry, Predefined, Surface, accelerator, item_text, key_only_ids};

    fn entries(json: &str) -> Vec<MenuEntry> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn predefined_items_are_named_as_the_ui_names_them() {
        let items = entries(
            r#"[{"id":"predefined.selectAll","label":"","predefined":"selectAll"},
            {"id":"predefined.bringAllToFront","label":"","predefined":"bringAllToFront"},
            {"id":"a","label":"A","reason":"why not","radio":true}]"#,
        );
        assert_eq!(items[0].predefined, Some(Predefined::SelectAll));
        assert_eq!(items[1].predefined, Some(Predefined::BringAllToFront));
        assert_eq!(items[2].predefined, None);
        assert!(serde_json::from_str::<Vec<MenuEntry>>(r#"[{"id":"x","label":"","predefined":"explode"}]"#).is_err());
    }

    #[test]
    fn menu_entries_read_the_uis_definition() {
        let items = entries(
            r#"[{"id":"file","label":"File","children":[
            {"id":"-","label":"","separator":true},
            {"id":"open","label":"Open","accelerator":"Cmd+O","shortcut":"Ctrl+O","enabled":false,"checked":true,"detail":"/x"}]}]"#,
        );
        let children = items[0].children.as_deref().unwrap();
        assert!(children[0].separator && children[0].accelerator.is_none());
        assert_eq!(children[1].accelerator.as_deref(), Some("Cmd+O"));
        assert_eq!((children[1].enabled, children[1].checked), (Some(false), Some(true)));
    }

    #[test]
    fn a_shortcut_no_accelerator_draws_is_text_after_the_label() {
        let items = entries(
            r#"[{"id":"a","label":"Copy path","shortcut":"Ctrl+C"},
            {"id":"b","label":"Open","detail":"/x","shortcut":"Ctrl+O"},
            {"id":"c","label":"Quit","accelerator":"Cmd+Q","shortcut":"Ctrl+Q"},
            {"id":"d","label":"Plain"}]"#,
        );
        let texts: Vec<String> = items
            .iter()
            .map(|i| item_text(i, Surface::MenuBar, accelerator(i).is_some()))
            .collect();
        assert_eq!(texts[2], "Quit");
        assert_eq!(texts[3], "Plain");
        // Only macOS draws the window's key (`chord`) in the key column; these have none.
        assert!(texts[0].starts_with("Copy path") && texts[0].ends_with("Ctrl+C"));
        assert!(texts[1].contains("/x") && texts[1].ends_with("Ctrl+O"));
    }

    #[test]
    fn the_windows_key_is_drawn_in_the_key_column_on_macos_and_as_text_elsewhere() {
        let items = entries(r#"[{"id":"a","label":"Go to definition","shortcut":"⌘B","chord":"Cmd+B"}]"#);
        let shown = accelerator(&items[0]);
        assert_eq!(shown, if cfg!(target_os = "macos") { Some("Cmd+B") } else { None });
        let bar = item_text(&items[0], Surface::MenuBar, shown.is_some());
        let context = item_text(&items[0], Surface::ContextMenu, shown.is_some());
        if cfg!(target_os = "macos") {
            assert_eq!(
                (bar.as_str(), context.as_str()),
                ("Go to definition", "Go to definition")
            );
        } else if cfg!(windows) {
            assert_eq!(
                (bar.as_str(), context.as_str()),
                ("Go to definition\t⌘B", "Go to definition\t⌘B")
            );
        } else {
            assert_eq!(
                (bar.as_str(), context.as_str()),
                ("Go to definition    ⌘B", "Go to definition")
            );
        }
    }

    #[test]
    fn only_items_that_draw_the_windows_key_ignore_the_event_it_raises() {
        let items = entries(
            r#"[{"id":"nav","label":"Navigate","children":[
            {"id":"next","label":"Next","shortcut":"⌥F","chord":"Alt+F"},
            {"id":"commit","label":"Next Commit","shortcut":"]","chord":"]"},
            {"id":"tab","label":"Next Tab","shortcut":"⇧⌘]","chord":"Cmd+Shift+]"},
            {"id":"sub","label":"More","children":[{"id":"deep","label":"Deep","chord":"F4"}]},
            {"id":"open","label":"Open","accelerator":"Cmd+O","chord":"Ctrl+O"},
            {"id":"plain","label":"Plain"}]}]"#,
        );
        let mut ids = std::collections::HashSet::new();
        key_only_ids(&items, &mut ids);
        let expected: &[&str] = if cfg!(target_os = "macos") {
            &["commit", "deep", "next", "tab"]
        } else {
            &[]
        };
        let mut ids: Vec<_> = ids.into_iter().collect();
        ids.sort();
        assert_eq!(ids, expected);
    }

    #[test]
    fn a_context_menu_gets_one_answer() {
        let state = ContextMenuChoice::default();
        let first = state.begin();
        assert!(state.answer(Some("a".into())));
        assert!(!state.answer(None));
        assert_eq!(first.recv().unwrap(), Some("a".into()));

        let abandoned = state.begin();
        let second = state.begin();
        assert!(abandoned.recv().is_err());
        assert!(state.answer(None));
        assert_eq!(second.recv().unwrap(), None);
    }
}
