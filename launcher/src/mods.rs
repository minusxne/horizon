//! An instance's Skyline mods, as the game finds and starts them (`docs/mods.md`).
//!
//! - Mods are `.hmod` files and mod folders (a folder holding `horizon-mod.toml`) in
//!   `skyline-mods/` beside `options.txt`.
//! - The ones turned on are the `mod.<id>:true` lines of `skyline-mods.txt`
//!   (`horizon_client::mod_list`); every mod is off until the player turns it on. The file's other
//!   lines (a mod's key bindings) are written back as they were.
//!
//! Adding copies a file or a whole folder in; removing turns the mod off and moves it to the
//! system's trash. Nothing is downloaded.
use std::fs;
use std::path::{Path, PathBuf};

/// The folder of mods in a game folder.
pub const DIR: &str = "skyline-mods";
/// Which mods are on.
pub const FILE: &str = "skyline-mods.txt";
const MANIFEST: &str = "horizon-mod.toml";
const PACKAGE: &str = "hmod";

#[derive(Clone, Debug, PartialEq)]
pub struct Mod {
    /// Its file or folder name in `skyline-mods/`: the id commands take.
    pub file: String,
    pub path: PathBuf,
    /// The manifest's id; None when the manifest can't be read (then it can't be turned on).
    pub id: Option<String>,
    /// `Name 1.0`, else the file name.
    pub title: String,
    pub description: String,
    /// A folder (a mod being made, or unpacked), not a `.hmod`.
    pub folder: bool,
    pub enabled: bool,
    /// Why the game would not load it.
    pub problem: Option<String>,
}

/// Whether `path` looks like a mod: a `.hmod` file, or a folder with `horizon-mod.toml`.
pub fn is_mod(path: &Path) -> bool {
    (path.is_file()
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(PACKAGE)))
        || path.join(MANIFEST).is_file()
}

/// The ids turned on in `skyline-mods.txt`.
fn enabled_ids(home: &Path) -> Vec<String> {
    fs::read_to_string(home.join(FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(_, v)| v.trim() == "true")
        .filter_map(|(k, _)| k.trim().strip_prefix("mod.").map(str::to_owned))
        .collect()
}

fn read(path: &Path, on: &[String]) -> Option<Mod> {
    let file = path.file_name()?.to_str()?.to_string();
    if file.starts_with('.') || !is_mod(path) {
        return None;
    }
    let (id, title, description, problem) = match horizon_mods::Package::read(path) {
        Ok(p) => (
            Some(p.manifest.id.clone()),
            format!("{} {}", p.manifest.name, p.manifest.version),
            p.manifest.description.clone(),
            None,
        ),
        Err(e) => (None, file.clone(), String::new(), Some(e)),
    };
    Some(Mod {
        enabled: id.as_ref().is_some_and(|id| on.contains(id)),
        folder: path.is_dir(),
        path: path.to_path_buf(),
        file,
        id,
        title,
        description,
        problem,
    })
}

/// The mods in the game folder `home`, by title.
pub fn list(home: &Path) -> Vec<Mod> {
    let on = enabled_ids(home);
    let mut mods: Vec<Mod> = fs::read_dir(home.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| read(&e.path(), &on))
        .collect();
    mods.sort_by_key(|m| m.title.to_lowercase());
    mods
}

/// The mod a command names: its file name, its id or its title (any case).
pub fn find(home: &Path, query: &str) -> Result<Mod, String> {
    let all = list(home);
    all.iter()
        .find(|m| m.file == query)
        .or_else(|| all.iter().find(|m| m.id.as_deref() == Some(query)))
        .or_else(|| all.iter().find(|m| m.title.eq_ignore_ascii_case(query)))
        .cloned()
        .ok_or_else(|| format!("No mod {query} in {}", home.join(DIR).display()))
}

/// Turns a mod on or off in `skyline-mods.txt`; every other line stays.
pub fn set_enabled(home: &Path, m: &Mod, on: bool) -> Result<(), String> {
    let id = m.id.as_ref().ok_or_else(|| {
        format!(
            "{} can't be turned on: {}",
            m.file,
            m.problem.as_deref().unwrap_or("its manifest can't be read")
        )
    })?;
    let path = home.join(FILE);
    let text = fs::read_to_string(&path).unwrap_or_default();
    let key = format!("mod.{id}");
    let mut out: Vec<String> = text
        .lines()
        .filter(|l| l.split_once(':').is_none_or(|(k, _)| k.trim() != key))
        .map(str::to_owned)
        .collect();
    if on {
        out.insert(0, format!("{key}:true"));
    }
    let mut text = out.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Copies a `.hmod` or a mod folder into `home`'s mods, under a name not taken yet. Returns it,
/// turned off, as the game would find it.
pub fn add(home: &Path, from: &Path) -> Result<Mod, String> {
    let name = from
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or("A mod has a name")?;
    if !is_mod(from) {
        return Err(format!(
            "{name} is not a Skyline mod: mods are .hmod files or folders with a {MANIFEST}."
        ));
    }
    let dir = home.join(DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (stem, ext) = match from.is_dir() {
        true => (name.clone(), String::new()),
        false => (
            Path::new(&name)
                .file_stem()
                .map_or(name.clone(), |s| s.to_string_lossy().into_owned()),
            format!(".{PACKAGE}"),
        ),
    };
    let target = (1..)
        .map(|n| match n {
            1 => dir.join(format!("{stem}{ext}")),
            n => dir.join(format!("{stem} ({n}){ext}")),
        })
        .find(|p| !p.exists())
        .unwrap();
    let copied = match from.is_dir() {
        true => crate::backend::copy_dir(from, &target),
        false => fs::copy(from, &target).map(|_| ()),
    };
    if let Err(e) = copied {
        let _ = fs::remove_dir_all(&target);
        let _ = fs::remove_file(&target);
        return Err(format!("Could not copy {}: {e}", from.display()));
    }
    read(&target, &enabled_ids(home))
        .ok_or_else(|| format!("{} is not a mod the game reads", target.display()))
}

/// Turns the mod off and moves it to the system's trash.
pub fn remove(home: &Path, m: &Mod) -> Result<(), String> {
    if m.enabled {
        set_enabled(home, m, false)?;
    }
    crate::desktop_seams::to_trash(&m.path)
        .map_err(|e| format!("Could not move {} to the trash: {e}", m.file))
}

#[cfg(test)]
#[path = "mods/tests.rs"]
mod tests;
