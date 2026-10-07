//! The native menu bar and context menus, which the UI defines.

use std::sync::Mutex;
use std::sync::mpsc;

use tauri::menu::{
    CheckMenuItemBuilder, ContextMenu, IsMenuItem, Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, Submenu,
    SubmenuBuilder,
};
use tauri::{AppHandle, LogicalPosition, Manager, State, Window, Wry};

/// One entry of a menu the UI defines (`MenuItem` in `ui/src/lib/ui/menuItem.ts`).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuEntry {
    id: String,
    label: String,
    /// Secondary text; a native menu has no column for it, so it follows the label.
    detail: Option<String>,
    /// The key that runs the item in the window, as `Ctrl+Alt+C`; the menu only shows it and the window handles the key.
    shortcut: Option<String>,
    /// A key the menu registers and handles itself, such as `Cmd+,`.
    accelerator: Option<String>,
    enabled: Option<bool>,
    checked: Option<bool>,
    #[serde(default)]
    separator: bool,
    children: Option<Vec<MenuEntry>>,
}

/// Where a menu is shown, which decides how its items show their shortcuts.
#[derive(Clone, Copy, PartialEq)]
enum Surface {
    MenuBar,
    ContextMenu,
}

/// The key a menu item shows and, in the menu bar, handles: its own accelerator or, in a context
/// menu on macOS, the window's shortcut. A context menu is not attached to the window, so its
/// accelerators only draw (right-aligned, in the OS's glyphs) and fire only while it is open.
/// Muda reads the shortcut text (`Cmd+B`, `Ctrl+Alt+Down`, `F4`, `Space`) as an accelerator.
fn accelerator(entry: &MenuEntry, surface: Surface) -> Option<&str> {
    if let Some(accelerator) = &entry.accelerator {
        return Some(accelerator);
    }
    if surface == Surface::ContextMenu && cfg!(target_os = "macos") {
        return entry.shortcut.as_deref();
    }
    None
}

/// The text of an item: its label, the detail after it, and the display shortcut where the menu
/// has no column for an accelerator. Windows right-aligns text after a tab as the shortcut column;
/// other context menus show no shortcut.
fn item_text(entry: &MenuEntry, surface: Surface) -> String {
    let mut text = entry.label.clone();
    if let Some(detail) = &entry.detail {
        text = format!("{text}  —  {detail}");
    }
    if let (Some(shortcut), None) = (&entry.shortcut, &entry.accelerator) {
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
    let enabled = entry.enabled.unwrap_or(true);
    if let Some(children) = &entry.children {
        return Ok(Box::new(submenu(app, &entry.label, enabled, children, surface)?));
    }
    let text = item_text(entry, surface);
    let accelerator = accelerator(entry, surface);
    // A shortcut that is no accelerator shows nothing rather than failing the menu.
    let build = |accelerator: Option<&str>| -> tauri::Result<Box<dyn IsMenuItem<Wry>>> {
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

/// Replaces the menu bar (macOS) with the UI's definition. The Edit menu is the host's: without
/// it the webview does not get ⌘C, ⌘V and ⌘Z.
#[tauri::command]
pub fn set_menu(app: AppHandle, items: Vec<MenuEntry>) -> Result<(), String> {
    let build = || -> tauri::Result<()> {
        let mut bar = MenuBuilder::new(&app);
        for (i, menu) in items.iter().enumerate() {
            let children = menu.children.as_deref().unwrap_or_default();
            bar = bar.item(&submenu(
                &app,
                &menu.label,
                menu.enabled.unwrap_or(true),
                children,
                Surface::MenuBar,
            )?);
            // After the app menu and File.
            if i == 1 {
                let edit = SubmenuBuilder::new(&app, "Edit")
                    .undo()
                    .redo()
                    .separator()
                    .cut()
                    .copy()
                    .paste()
                    .select_all()
                    .build()?;
                bar = bar.item(&edit);
            }
        }
        app.set_menu(bar.build()?)?;
        Ok(())
    };
    build().map_err(|e| e.to_string())
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
    use super::{ContextMenuChoice, MenuEntry, Surface, accelerator, item_text};

    fn entries(json: &str) -> Vec<MenuEntry> {
        serde_json::from_str(json).unwrap()
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
    fn the_menu_bar_shows_a_shortcut_as_text_unless_the_item_registers_an_accelerator() {
        let items = entries(
            r#"[{"id":"a","label":"Copy path","shortcut":"Ctrl+C"},
            {"id":"b","label":"Open","detail":"/x","shortcut":"Ctrl+O"},
            {"id":"c","label":"Quit","accelerator":"Cmd+Q","shortcut":"Ctrl+Q"},
            {"id":"d","label":"Plain"}]"#,
        );
        let texts: Vec<String> = items.iter().map(|i| item_text(i, Surface::MenuBar)).collect();
        assert!(texts[0].starts_with("Copy path") && texts[0].ends_with("Ctrl+C"));
        assert!(texts[1].contains("/x") && texts[1].ends_with("Ctrl+O"));
        assert_eq!(texts[2], "Quit");
        assert_eq!(texts[3], "Plain");
    }

    #[test]
    fn a_context_menu_item_keeps_its_shortcut_out_of_the_label_except_after_a_tab_on_windows() {
        let items = entries(r#"[{"id":"a","label":"Go to definition","shortcut":"Cmd+B"}]"#);
        let text = item_text(&items[0], Surface::ContextMenu);
        if cfg!(windows) {
            assert_eq!(text, "Go to definition\tCmd+B");
        } else {
            assert_eq!(text, "Go to definition");
        }
        let shown = accelerator(&items[0], Surface::ContextMenu);
        assert_eq!(shown, if cfg!(target_os = "macos") { Some("Cmd+B") } else { None });
        assert_eq!(accelerator(&items[0], Surface::MenuBar), None);
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
