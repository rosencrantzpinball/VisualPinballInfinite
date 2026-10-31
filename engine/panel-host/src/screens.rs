//! What the app's `table_settings` (compiled in here, `main.rs`) asks of the app's `screens`: the
//! settings file written as the app writes it.

/// The settings file written as `after`, if that differs from what it was (`before`): the file
/// before kept as `Settings.previous.ini`, the new one written beside it and moved into place, so a
/// failed write leaves the old one (the app's `screens::write_settings`).
pub fn write_settings(path: &std::path::Path, before: &str, after: &str) -> Result<(), String> {
    if after == before {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("The settings folder couldn't be made: {e}"))?;
    }
    if !before.is_empty() {
        std::fs::write(path.with_file_name("Settings.previous.ini"), before).map_err(|e| format!("The settings couldn't be kept before changing them: {e}"))?;
    }
    let new = path.with_extension("ini.new");
    std::fs::write(&new, after).map_err(|e| format!("The settings couldn't be written: {e}"))?;
    std::fs::rename(&new, path).map_err(|e| format!("The settings couldn't be written: {e}"))
}
