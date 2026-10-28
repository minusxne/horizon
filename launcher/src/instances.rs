//! Instances: separate game folders, each with its own worlds, options and packs.
//!
//! Nothing that exists moves. The game's home (`backend::game_home`) stays the default instance,
//! "Skyline", with its saves and options where they are. New instances get `instances/<slug>/`
//! in the game's home. What the player chooses about an instance (its name, icon, group) is kept
//! in `instances.json` in the launcher's data folder, so renaming one never moves a folder.
use crate::backend;
use crate::launch::Launch;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The default instance's id: the game's home itself.
pub const DEFAULT: &str = "horizon";
const FILE: &str = "instances.json";
/// Where new instances live, in the game's home.
const OWN: &str = "instances";

/// A folder name for `name`: its letters and digits in lower case, words joined by `-`.
pub fn slug(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if words.is_empty() {
        "pack".into()
    } else {
        words.join("-")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The game's home: what every player had before instances.
    Default,
    /// Made in the launcher (or imported from a .zip): `instances/<slug>/`.
    Own,
}

/// What an instance shows in the sidebar: a block or item texture from the player's prepared
/// files (never shipped), or a colour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Icon {
    /// A texture name: `grass_block` (block/ is looked in first, then item/).
    Texture(String),
    /// `#rrggbb`.
    Colour(String),
}

impl Icon {
    /// `#3fa7d6` is a colour; a texture's name is lowercase letters, digits and `_`.
    pub fn parse(text: &str) -> Result<Icon, String> {
        let text = text.trim();
        if let Some(hex) = text.strip_prefix('#') {
            if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(Icon::Colour(format!("#{}", hex.to_ascii_lowercase())));
            }
        } else if !text.is_empty()
            && text
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Ok(Icon::Texture(text.into()));
        }
        Err(format!(
            "{text}: give a block or item name (grass_block) or a colour (#3fa7d6)"
        ))
    }

    /// The texture's file in prepared resources, if it is a texture they hold.
    pub fn texture(&self, resources: &Path) -> Option<PathBuf> {
        let Icon::Texture(name) = self else {
            return None;
        };
        let textures = resources.join("assets/minecraft/textures");
        ["block", "item"]
            .iter()
            .map(|kind| textures.join(kind).join(format!("{name}.png")))
            .find(|p| p.is_file())
    }
}

/// The player's choices about one instance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// The Skyline version this instance plays, kept when newer ones are installed. None: the
    /// launcher's own version, which follows updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Seconds played in it, every run started by the launcher (the window or `play`) added up.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub played_secs: u64,
    /// When its last run ended, in seconds since 1970.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played: Option<u64>,
    /// What runs around the game: a pre-launch command, a wrapper, variables (`launch`).
    #[serde(default, skip_serializing_if = "Launch::is_empty")]
    pub launch: Launch,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// `instances.json`: each instance's choices, by id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    instances: BTreeMap<String, Meta>,
}

impl Store {
    fn load(data: &Path) -> Store {
        fs::read_to_string(data.join(FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    fn save(&self, data: &Path) -> Result<(), String> {
        fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
        let text = serde_json::to_string_pretty(self).unwrap();
        fs::write(data.join(FILE), text).map_err(|e| format!("{FILE}: {e}"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    /// `horizon` (the default instance, kept from before the rename) or an own instance's folder.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// The folder its tools work in: the game's home for the default instance, else its own.
    pub work: PathBuf,
    /// The game folder it plays in: saves, options.txt, packs.
    pub home: PathBuf,
    pub icon: Option<Icon>,
    pub group: Option<String>,
    /// Its pinned Skyline version, if any (`Meta::version`).
    pub version: Option<String>,
    pub played: Duration,
    pub last_played: Option<SystemTime>,
    /// What runs around the game (`launch::wrap`).
    pub launch: Launch,
}

impl Instance {
    /// The installed version this instance plays: its pinned one, else `requested` (the
    /// command line's `--version`) or the launcher's (`backend::resolve_version`).
    pub fn resolve_version(&self, requested: Option<&str>) -> Result<String, String> {
        match (requested, &self.version) {
            (None, Some(pinned)) => backend::resolve_version(Some(pinned)).map_err(|_| {
                format!(
                    "{} plays {pinned}, which is not installed: install it, or skyline-launcher \
                     instance version \"{}\" latest",
                    self.name, self.name
                )
            }),
            _ => backend::resolve_version(requested),
        }
    }
}

/// The instances, by the default game home and the launcher's data folder: Skyline first, then
/// by group and name.
pub fn list() -> Vec<Instance> {
    let found = list_in(&backend::game_home(), &backend::data_dir());
    // Once per run: game folders from before the rename get Skyline's file names.
    static MIGRATED: std::sync::Once = std::sync::Once::new();
    MIGRATED.call_once(|| {
        for i in &found {
            horizon_core::rebrand::migrate_game_dir(&i.home);
        }
    });
    found
}

fn list_in(game_home: &Path, data: &Path) -> Vec<Instance> {
    let store = Store::load(data);
    let mut found = vec![(
        DEFAULT.to_string(),
        "Skyline".to_string(),
        Kind::Default,
        game_home.to_path_buf(),
        game_home.to_path_buf(),
    )];
    let mut own: Vec<_> = fs::read_dir(game_home.join(OWN))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    own.sort();
    for slug in own {
        let dir = game_home.join(OWN).join(&slug);
        found.push((slug.clone(), slug, Kind::Own, dir.clone(), dir));
    }
    let mut out: Vec<Instance> = found
        .into_iter()
        .map(|(id, name, kind, work, home)| {
            let meta = store.instances.get(&id).cloned().unwrap_or_default();
            Instance {
                name: meta.name.unwrap_or(name),
                icon: meta.icon,
                group: meta.group,
                version: meta.version,
                played: Duration::from_secs(meta.played_secs),
                last_played: meta
                    .last_played
                    .map(|s| UNIX_EPOCH + Duration::from_secs(s)),
                launch: meta.launch,
                id,
                kind,
                work,
                home,
            }
        })
        .collect();
    out[1..].sort_by_key(|i| (i.group.clone(), i.name.to_lowercase()));
    out
}

/// The instance an id or a name (any case) names.
pub fn find(query: &str) -> Result<Instance, String> {
    find_in(&list(), query)
}

fn find_in(all: &[Instance], query: &str) -> Result<Instance, String> {
    let q = query.trim();
    all.iter()
        .find(|i| i.id == q)
        .or_else(|| all.iter().find(|i| i.name.eq_ignore_ascii_case(q)))
        .cloned()
        .ok_or_else(|| format!("No instance {q}: skyline-launcher instance list shows them"))
}

/// The instance Play starts: the one picked in the launcher, else Skyline.
pub fn selected() -> Instance {
    let all = list();
    backend::Settings::load()
        .instance
        .and_then(|id| all.iter().find(|i| i.id == id).cloned())
        .unwrap_or_else(|| all[0].clone())
}

pub fn select(query: &str) -> Result<Instance, String> {
    let instance = find(query)?;
    let mut settings = backend::Settings::load();
    settings.instance = (instance.kind != Kind::Default).then(|| instance.id.clone());
    settings.save();
    Ok(instance)
}

/// What a copy of a game folder leaves out: other instances, logs and crash reports, caches, and
/// what features Skyline no longer has left behind (sessions, data collection, the mod browser,
/// modpacks).
const NOT_COPIED: &[&str] = &[
    OWN,
    "ports",
    "skyline-packs",
    "logs",
    "crash-reports",
    "sessions",
    "telemetry",
    "mod-browser",
    ".skyline-cache",
];

/// Files of the player, not of one instance, that a new instance starts with: the keys and video
/// settings, and the offline name (so the player stays the same player in its worlds).
const PLAYER_FILES: &[&str] = &["options.txt", "skyline-player.txt"];

/// Creates an instance named `name`: empty but for the player's options and offline name, or a
/// copy of `from`'s game folder (its worlds, options and packs). Nothing of `from` changes.
pub fn create(name: &str, from: Option<&str>) -> Result<Instance, String> {
    let from = from.map(find).transpose()?;
    create_in(
        &backend::game_home(),
        &backend::data_dir(),
        name,
        from.as_ref(),
    )
}

fn create_in(
    game_home: &Path,
    data: &Path,
    name: &str,
    from: Option<&Instance>,
) -> Result<Instance, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("An instance needs a name.".into());
    }
    let all = list_in(game_home, data);
    if all.iter().any(|i| i.name.eq_ignore_ascii_case(name)) {
        return Err(format!("There is already an instance called {name}."));
    }
    let base = slug(name);
    let taken = |s: &str| s == DEFAULT || game_home.join(OWN).join(s).exists();
    let slug = (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|s| !taken(s))
        .unwrap();
    let dir = game_home.join(OWN).join(&slug);
    let made = match from {
        Some(from) => copy_home(&from.home, &dir),
        None => fs::create_dir_all(&dir).and_then(|()| {
            for f in PLAYER_FILES {
                let own = game_home.join(f);
                if own.is_file() {
                    fs::copy(own, dir.join(f))?;
                }
            }
            Ok(())
        }),
    };
    if let Err(e) = made {
        let _ = fs::remove_dir_all(&dir);
        return Err(format!("Could not make {}: {e}", dir.display()));
    }
    let mut store = Store::load(data);
    let meta = store.instances.entry(slug.clone()).or_default();
    meta.name = (name != slug).then(|| name.to_string());
    meta.icon = from.and_then(|f| f.icon.clone());
    meta.group = from.and_then(|f| f.group.clone());
    store.save(data)?;
    find_in(&list_in(game_home, data), &slug)
}

fn copy_home(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if NOT_COPIED.iter().any(|n| name == *n) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Changes what the player chose about an instance: its name, icon or group. No folder moves.
pub fn edit(query: &str, change: impl FnOnce(&mut Meta)) -> Result<Instance, String> {
    edit_in(&backend::game_home(), &backend::data_dir(), query, change)
}

fn edit_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    change: impl FnOnce(&mut Meta),
) -> Result<Instance, String> {
    let all = list_in(game_home, data);
    let instance = find_in(&all, query)?;
    let mut store = Store::load(data);
    let meta = store.instances.entry(instance.id.clone()).or_default();
    change(meta);
    meta.name = meta.name.take().map(|n| n.trim().to_string());
    meta.group = meta
        .group
        .take()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());
    if let Some(name) = &meta.name {
        if name.is_empty() {
            return Err("An instance needs a name.".into());
        }
        // Only a new name is checked: an import can share a name, and changing anything
        // else about it must still work.
        let renamed = !name.eq_ignore_ascii_case(&instance.name);
        if renamed
            && all
                .iter()
                .any(|i| i.id != instance.id && i.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("There is already an instance called {name}."));
        }
    }
    store.save(data)?;
    find_in(&list_in(game_home, data), &instance.id)
}

/// Deletes an own instance: its game folder goes to the system's trash (so a mistake can be
/// undone there) and its choices leave `instances.json`. The default instance is the game's
/// home and is never deleted. Returns the folder that was moved.
pub fn delete(query: &str) -> Result<PathBuf, String> {
    let data = backend::data_dir();
    let home = delete_in(&backend::game_home(), &data, query, |dir| {
        trash::delete(dir).map_err(|e| e.to_string())
    })?;
    let mut settings = backend::Settings::load();
    if settings.instance.is_some()
        && !list()
            .iter()
            .any(|i| Some(&i.id) == settings.instance.as_ref())
    {
        settings.instance = None;
        settings.save();
    }
    Ok(home)
}

fn delete_in(
    game_home: &Path,
    data: &Path,
    query: &str,
    remove: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let instance = find_in(&list_in(game_home, data), query)?;
    if instance.kind == Kind::Default {
        return Err(format!(
            "{} is the game's own folder and can't be deleted.",
            instance.name
        ));
    }
    if instance.home.exists() {
        remove(&instance.home)
            .map_err(|e| format!("Could not delete {}: {e}", instance.home.display()))?;
    }
    let mut store = Store::load(data);
    if store.instances.remove(&instance.id).is_some() {
        store.save(data)?;
    }
    Ok(instance.home)
}

/// Adds a finished run of the game to the instance's play time (by id: a renamed instance keeps
/// it).
pub fn record_play(id: &str, played: Duration) -> Result<(), String> {
    record_play_in(&backend::data_dir(), id, played, SystemTime::now())
}

fn record_play_in(
    data: &Path,
    id: &str,
    played: Duration,
    ended: SystemTime,
) -> Result<(), String> {
    let mut store = Store::load(data);
    let meta = store.instances.entry(id.to_string()).or_default();
    meta.played_secs = meta.played_secs.saturating_add(played.as_secs());
    meta.last_played = ended.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
    store.save(data)
}

/// "12 min", "3 h 05 min", "140 h": the play time as players read it.
pub fn play_time(played: Duration) -> String {
    let minutes = played.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => crate::i18n::tr("under a minute").into(),
        (0, m) => format!("{m} min"),
        (h, m) if h < 100 => format!("{h} h {m:02} min"),
        (h, _) => format!("{h} h"),
    }
}

/// Turns the game's command into this instance: its own game folder. options.txt and the
/// offline name follow it in from the game's home when it has none yet.
pub fn apply(cmd: &mut Command, instance: &Instance) -> Result<(), String> {
    match instance.kind {
        Kind::Default => return Ok(()),
        Kind::Own => {
            fs::create_dir_all(&instance.home)
                .map_err(|e| format!("{}: {e}", instance.home.display()))?;
            cmd.current_dir(&instance.home)
                .env("SKYLINE_HOME", &instance.home);
        }
    }
    follow_player(&backend::game_home(), &instance.home);
    Ok(())
}

fn follow_player(game_home: &Path, home: &Path) {
    for f in PLAYER_FILES {
        if !home.join(f).exists() && game_home.join(f).is_file() {
            let _ = fs::copy(game_home.join(f), home.join(f));
        }
    }
}

#[cfg(test)]
#[path = "instances/tests.rs"]
mod tests;
