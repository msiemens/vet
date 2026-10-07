//! The native menu bar, which the UI defines.

use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Wry};

/// One entry of the main menu the UI defines (`MenuItem` in `ui/src/lib/ui/menuItem.ts`).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuEntry {
    id: String,
    label: String,
    /// Secondary text; a native menu has no column for it, so it follows the label.
    detail: Option<String>,
    /// An accelerator such as `Cmd+,`.
    shortcut: Option<String>,
    enabled: Option<bool>,
    checked: Option<bool>,
    #[serde(default)]
    separator: bool,
    children: Option<Vec<MenuEntry>>,
}

fn submenu(app: &AppHandle, label: &str, enabled: bool, entries: &[MenuEntry]) -> tauri::Result<Submenu<Wry>> {
    let mut builder = SubmenuBuilder::new(app, label).enabled(enabled);
    for entry in entries {
        if entry.separator {
            builder = builder.separator();
        } else if let Some(children) = &entry.children {
            builder = builder.item(&submenu(app, &entry.label, entry.enabled.unwrap_or(true), children)?);
        } else {
            let text = match &entry.detail {
                Some(detail) => format!("{}  —  {detail}", entry.label),
                None => entry.label.clone(),
            };
            let enabled = entry.enabled.unwrap_or(true);
            if let Some(checked) = entry.checked {
                let mut item = CheckMenuItemBuilder::with_id(&entry.id, text)
                    .checked(checked)
                    .enabled(enabled);
                if let Some(shortcut) = &entry.shortcut {
                    item = item.accelerator(shortcut);
                }
                builder = builder.item(&item.build(app)?);
            } else {
                let mut item = MenuItemBuilder::with_id(&entry.id, text).enabled(enabled);
                if let Some(shortcut) = &entry.shortcut {
                    item = item.accelerator(shortcut);
                }
                builder = builder.item(&item.build(app)?);
            }
        }
    }
    builder.build()
}

/// Replaces the menu bar (macOS) with the UI's definition. The Edit menu is the host's: without
/// it the webview does not get ⌘C, ⌘V and ⌘Z.
#[tauri::command]
pub fn set_menu(app: AppHandle, items: Vec<MenuEntry>) -> Result<(), String> {
    let build = || -> tauri::Result<()> {
        let mut bar = MenuBuilder::new(&app);
        for (i, menu) in items.iter().enumerate() {
            let children = menu.children.as_deref().unwrap_or_default();
            bar = bar.item(&submenu(&app, &menu.label, menu.enabled.unwrap_or(true), children)?);
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

#[cfg(test)]
mod tests {
    use super::MenuEntry;

    #[test]
    fn menu_entries_read_the_uis_definition() {
        let json = r#"[{"id":"file","label":"File","children":[
            {"id":"-","label":"","separator":true},
            {"id":"open","label":"Open","shortcut":"Cmd+O","enabled":false,"checked":true,"detail":"/x"}]}]"#;
        let items: Vec<MenuEntry> = serde_json::from_str(json).unwrap();
        let children = items[0].children.as_deref().unwrap();
        assert!(children[0].separator && children[0].shortcut.is_none());
        assert_eq!(children[1].shortcut.as_deref(), Some("Cmd+O"));
        assert_eq!((children[1].enabled, children[1].checked), (Some(false), Some(true)));
    }
}
