//! The list of recently opened repositories, as the backend sees their paths.

use tauri::Manager;

const RECENT_LIMIT: usize = 10;

/// `path` first, without duplicates, at most `RECENT_LIMIT` entries.
fn with_recent(mut list: Vec<String>, path: String) -> Vec<String> {
    list.retain(|entry| *entry != path);
    list.insert(0, path);
    list.truncate(RECENT_LIMIT);
    list
}

/// In the app config directory.
fn recent_file(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(dir.join("recent-repositories.json"))
}

fn read_recent(app: &tauri::AppHandle) -> Vec<String> {
    recent_file(app)
        .ok()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_recent(app: &tauri::AppHandle, list: &[String]) -> Result<(), String> {
    let file = recent_file(app)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(&file, text).map_err(|e| format!("cannot write {}: {e}", file.display()))
}

#[tauri::command]
pub fn recent_repositories(app: tauri::AppHandle) -> Vec<String> {
    read_recent(&app)
}

#[tauri::command]
pub fn remember_repository(app: tauri::AppHandle, path: String) -> Result<(), String> {
    write_recent(&app, &with_recent(read_recent(&app), path))
}

#[tauri::command]
pub fn forget_repository(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let mut list = read_recent(&app);
    list.retain(|entry| *entry != path);
    write_recent(&app, &list)
}

#[cfg(test)]
mod tests {
    use super::with_recent;

    #[test]
    fn recent_list_is_most_recent_first_without_duplicates_and_capped() {
        let list: Vec<String> = (0..10).map(|i| format!("/r{i}")).collect();
        let list = with_recent(list, "/r5".to_owned());
        assert_eq!(list[0], "/r5");
        assert_eq!(list.len(), 10);
        assert_eq!(list.iter().filter(|p| *p == "/r5").count(), 1);
        assert_eq!(with_recent(list, "/new".to_owned()).len(), 10);
    }
}
