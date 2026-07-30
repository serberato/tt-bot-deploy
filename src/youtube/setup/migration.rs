//! Migration of legacy YouTube tools directory.

use std::path::Path;

use super::paths::BGUTIL_VERSION_FILE;

/// Everything our installer puts into the tools dir. Used by the migration to
/// move exactly our items and nothing else.
pub(crate) fn tool_item_names() -> [&'static str; 4] {
    if cfg!(windows) {
        [
            "yt-dlp.exe",
            "bgutil-pot.exe",
            "yt-dlp-plugins",
            BGUTIL_VERSION_FILE,
        ]
    } else {
        [
            "yt-dlp",
            "bgutil-pot",
            "yt-dlp-plugins",
            BGUTIL_VERSION_FILE,
        ]
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), &dest)?;
        }
    }
    Ok(())
}

/// One-time move of our tools from a legacy exe-side `lib/` into the new
/// location. Runs only when the legacy dir is provably ours.
pub fn migrate_tools_dir(legacy: &Path, target: &Path) -> bool {
    if legacy == target || !legacy.join(BGUTIL_VERSION_FILE).is_file() {
        return false;
    }
    for name in tool_item_names() {
        let src = legacy.join(name);
        if !src.exists() {
            continue;
        }
        let dest = target.join(name);
        let copied = if src.is_dir() {
            copy_dir_recursive(&src, &dest)
        } else {
            std::fs::create_dir_all(target)
                .and_then(|()| std::fs::copy(&src, &dest).map(|_| ()))
        };
        if let Err(e) = copied {
            tracing::warn!(
                "YouTube tools migration aborted (copying {name}: {e}); staying in {}",
                legacy.display()
            );
            return false;
        }
    }
    for name in tool_item_names() {
        let src = legacy.join(name);
        let removed = if src.is_dir() {
            std::fs::remove_dir_all(&src)
        } else if src.exists() {
            std::fs::remove_file(&src)
        } else {
            Ok(())
        };
        if let Err(e) = removed {
            tracing::warn!("Could not remove migrated {name} from old tools dir: {e}");
        }
    }
    let _ = std::fs::remove_dir(legacy);
    tracing::info!(
        "Moved YouTube tools from {} to {}",
        legacy.display(),
        target.display()
    );
    true
}

/// Move a legacy exe-side tools install to the XDG data dir (Linux only).
pub fn migrate_legacy_tools() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(exe_dir) = exe.parent() else {
        return;
    };
    let Some(data) = dirs::data_dir() else {
        return;
    };
    migrate_tools_dir(
        &exe_dir.join("lib"),
        &data.join("ttspotify").join("lib"),
    );
}
