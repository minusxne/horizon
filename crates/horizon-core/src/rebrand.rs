//! The move to Skyline's names. Folders and files that older builds wrote under the previous
//! brand are renamed once, the first time Skyline looks for them: only when the old one exists
//! and the new one doesn't, so nothing is ever overwritten and running it again does nothing.
use std::{fs, io, path::Path};

/// The game's own files and folders in a game folder (the one with `options.txt`): the previous
/// name, then Skyline's.
pub const GAME_FILES: [(&str, &str); 9] = [
    ("horizon-player.txt", "skyline-player.txt"),
    ("horizon-graphics.txt", "skyline-graphics.txt"),
    ("horizon-extras.txt", "skyline-extras.txt"),
    ("horizon.txt", "skyline.txt"),
    ("horizon-controller.txt", "skyline-controller.txt"),
    ("horizon-mods.txt", "skyline-mods.txt"),
    ("horizon-mods", "skyline-mods"),
    (".horizon-cache", ".skyline-cache"),
    ("horizon-packs", "skyline-packs"),
];

/// The data folders in the platform data folder: the game's and the launcher's.
pub const DATA_DIRS: [(&str, &str); 2] = [
    ("horizon", "skyline"),
    ("horizon-launcher", "skyline-launcher"),
];

/// Renames `parent/old` to `parent/new` when only the old one is there. Whether it moved.
pub fn rename_once(parent: &Path, old: &str, new: &str) -> io::Result<bool> {
    let (from, to) = (parent.join(old), parent.join(new));
    if fs::symlink_metadata(&to).is_ok() || fs::symlink_metadata(&from).is_err() {
        return Ok(false);
    }
    fs::rename(&from, &to)?;
    Ok(true)
}

/// Moves prepared resources in `root` (a build or version folder) to Skyline's names:
/// `.local/horizon-client/<version>` to `.local/skyline/<version>`, and each version's
/// `horizon-resources.json` marker to `skyline-resources.json`.
pub fn migrate_resources(root: &Path) {
    let local = root.join(".local");
    let _ = rename_once(&local, "horizon-client", "skyline");
    if let Ok(versions) = fs::read_dir(local.join("skyline")) {
        for v in versions.flatten() {
            let _ = rename_once(
                &v.path(),
                "horizon-resources.json",
                "skyline-resources.json",
            );
        }
    }
}

/// Moves a game folder's files to Skyline's names. Errors are skipped: a file that can't move
/// stays where it was and the game starts with its defaults.
pub fn migrate_game_dir(dir: &Path) {
    for (old, new) in GAME_FILES {
        let _ = rename_once(dir, old, new);
    }
}

/// Moves the data folders in `data` (the platform data folder) to Skyline's names.
pub fn migrate_data_dirs(data: &Path) {
    for (old, new) in DATA_DIRS {
        let _ = rename_once(data, old, new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("skyline-rebrand-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn old_files_move_once_and_never_overwrite() {
        let dir = scratch("files");
        fs::write(dir.join("horizon-player.txt"), "name:Alex\n").unwrap();
        fs::write(dir.join("horizon-graphics.txt"), "old").unwrap();
        fs::write(dir.join("skyline-graphics.txt"), "new").unwrap();
        fs::create_dir(dir.join("horizon-mods")).unwrap();
        migrate_game_dir(&dir);
        assert_eq!(
            fs::read_to_string(dir.join("skyline-player.txt")).unwrap(),
            "name:Alex\n"
        );
        assert!(!dir.join("horizon-player.txt").exists());
        assert_eq!(
            fs::read_to_string(dir.join("skyline-graphics.txt")).unwrap(),
            "new"
        );
        assert!(dir.join("horizon-graphics.txt").exists());
        assert!(dir.join("skyline-mods").is_dir());
        migrate_game_dir(&dir);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resources_move_with_their_marker() {
        let dir = scratch("resources");
        fs::create_dir_all(dir.join(".local/horizon-client/26.3")).unwrap();
        fs::write(
            dir.join(".local/horizon-client/26.3/horizon-resources.json"),
            "{}",
        )
        .unwrap();
        migrate_resources(&dir);
        assert!(
            dir.join(".local/skyline/26.3/skyline-resources.json")
                .is_file()
        );
        assert!(!dir.join(".local/horizon-client").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn data_dirs_move() {
        let dir = scratch("data");
        fs::create_dir(dir.join("horizon")).unwrap();
        fs::create_dir(dir.join("horizon-launcher")).unwrap();
        migrate_data_dirs(&dir);
        assert!(dir.join("skyline").is_dir() && dir.join("skyline-launcher").is_dir());
        assert!(!dir.join("horizon").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
