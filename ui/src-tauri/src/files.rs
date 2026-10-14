//! Files the shell keeps in the app's config directory.

use std::io::Write;
use std::path::Path;

/// Replaces `file` whole: a reader sees the old text or the new, never a torn write.
pub fn write_atomic(file: &Path, text: &str) -> std::io::Result<()> {
    let dir = file
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(text.as_bytes())?;
    temp.persist(file).map_err(|e| e.error)?;
    Ok(())
}
