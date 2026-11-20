//! Shared preflight checks for prepared game resources and the previous client launch.

use super::Settings;

/// Every launch checks the prepared game files before opening a window or
/// starting the server. `get` is the page a hook sends a player to.
pub(super) fn missing_resources(
    root: &std::path::Path,
    get: Option<horizon_hooks::GetTheGame>,
) -> Option<String> {
    let shown = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    if !root.join("horizon-resources.json").is_file() {
        let launcher = get.map_or(String::new(), |get| {
            format!(
                "Horizon is started from its launcher: download it at {} (Download). It needs \
                 your own Minecraft: Java Edition; the launcher signs you in and prepares the \
                 game files.\nDevelopers: ",
                get.address
            )
        });
        return Some(format!(
            "Horizon's prepared game files are missing: no horizon-resources.json in {}\n\
             {launcher}horizon-prepare --prepare, or pass --resources <folder>",
            shown.display(),
        ));
    }
    damaged_prepared(root).map(|problem| {
        format!(
            "Horizon's prepared game files are damaged: {problem} in {}\n\
             Prepare them again with the launcher, or: horizon-prepare --prepare",
            shown.display()
        )
    })
}

/// Detect incomplete JSON left by a killed preparation or interrupted copy.
pub(super) fn damaged_prepared(root: &std::path::Path) -> Option<String> {
    let manifest = root.join("horizon-resources.json");
    let bytes = std::fs::read(&manifest).ok()?;
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Some("horizon-resources.json is empty".into());
    }
    if serde_json::from_slice::<serde_json::Value>(&bytes).is_err() {
        return Some("horizon-resources.json is not valid JSON".into());
    }
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.extension() != Some(std::ffi::OsStr::new("json")) {
            continue;
        }
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() == 0) {
            return Some(format!("{} is empty", entry.file_name().to_string_lossy()));
        }
    }
    None
}

/// Reset fullscreen after an unclean client launch, matching vanilla's recovery.
pub(super) fn unclean_start_recovery(settings: &mut Settings) -> bool {
    let unclean = matches!(
        settings.file.get("startedCleanly"),
        Some(horizon_client::options_file::V::Bool(false))
    );
    if unclean {
        settings.fullscreen = false;
        settings.file.fullscreen_resolution = None;
    }
    unclean
}
