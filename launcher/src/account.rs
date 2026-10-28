//! Who plays: offline names only. accounts.json keeps the names the player made and the one that
//! plays; with none picked, the game's own name plays (the one it made up and keeps in
//! `skyline-player.txt`). Skyline has no sign-in: every name is an offline profile, so its UUID
//! comes from the name and a world's player data follows the name, not the person.
use std::path::{Path, PathBuf};

/// What the game keeps its offline name in, beside `options.txt` (the game's `offline_name.rs`).
pub const PLAYER_FILE: &str = "skyline-player.txt";
const KEY: &str = "name:";

/// Said wherever a name can change: the worlds know a player by the UUID the name makes.
pub const NAME_CHANGES_UUID: &str = "A new name is a new player to your worlds: the inventory and position saved under the old name stay with the old name.";

/// accounts.json: the offline names and the one that plays.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Accounts {
    /// Offline names the player keeps, to switch between (`--name`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offline: Vec<String>,
    /// The offline name that plays; none: the name the game made up (`skyline-player.txt`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_selected: Option<String>,
}

/// Who plays, as the quick switch lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Player {
    /// An offline name.
    Offline(String),
    /// The name the game made up.
    GameName,
}

/// Whether `name` can be a player name: what the game accepts (`lan::conn::valid_name`), 1 to 16
/// printable ASCII characters without spaces.
pub fn valid_offline_name(name: &str) -> bool {
    (1..=16).contains(&name.len()) && name.chars().all(|c| c as u32 > 32 && (c as u32) < 127)
}

/// Why a name was refused, for the field that took it.
pub fn name_problem(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        Some("Type a name.".into())
    } else if name.len() > 16 {
        Some("A name is at most 16 characters.".into())
    } else if !valid_offline_name(name) {
        Some(
            "A name has no spaces and only plain letters, digits and symbols (A-Z, 0-9, _).".into(),
        )
    } else {
        None
    }
}

/// The name kept in a game folder's `skyline-player.txt`, if it has a valid one.
pub fn game_name(home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join(PLAYER_FILE)).ok()?;
    text.lines()
        .find_map(|l| l.trim_end_matches('\r').strip_prefix(KEY))
        .filter(|n| valid_offline_name(n))
        .map(str::to_owned)
}

/// Keeps `name` as a game folder's name (`skyline-player.txt`), so the game plays it even when
/// started outside Skyline's launcher. Other lines in the file stay.
pub fn write_game_name(home: &Path, name: &str) -> Result<(), String> {
    if !valid_offline_name(name) {
        return Err(format!("{name}: not a valid player name"));
    }
    let path = home.join(PLAYER_FILE);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| !l.starts_with(KEY))
        .map(str::to_owned)
        .collect();
    lines.insert(0, format!("{KEY}{name}"));
    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    std::fs::write(&path, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

impl Accounts {
    fn path() -> PathBuf {
        crate::backend::data_dir().join("accounts.json")
    }
    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text + "\n").map_err(|e| e.to_string())
    }

    /// Who plays now.
    pub fn player(&self) -> Player {
        match &self.offline_selected {
            Some(name) => Player::Offline(name.clone()),
            None => Player::GameName,
        }
    }

    /// Plays as `who`: an offline name (kept, once in any case) or the game's own name.
    pub fn play_as(&mut self, who: Player) -> Result<(), String> {
        match who {
            Player::Offline(name) => {
                let name = self.add_offline(&name)?;
                self.offline_selected = Some(name);
            }
            Player::GameName => self.offline_selected = None,
        }
        Ok(())
    }

    /// Keeps an offline name (once, in any case); returns it as kept.
    pub fn add_offline(&mut self, name: &str) -> Result<String, String> {
        let name = name.trim();
        if let Some(why) = name_problem(name) {
            return Err(why);
        }
        if let Some(kept) = self.offline.iter().find(|n| n.eq_ignore_ascii_case(name)) {
            return Ok(kept.clone());
        }
        self.offline.push(name.to_string());
        Ok(name.to_string())
    }

    /// Forgets an offline name; if it was playing, the game's own name plays.
    pub fn remove_offline(&mut self, name: &str) -> Result<(), String> {
        let before = self.offline.len();
        self.offline.retain(|n| !n.eq_ignore_ascii_case(name));
        if self.offline.len() == before {
            return Err(format!("No offline name {name}."));
        }
        if self
            .offline_selected
            .as_ref()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
        {
            self.offline_selected = None;
        }
        Ok(())
    }

    /// The game's `--name` for this run: the offline name, when one plays.
    pub fn name_arg(&self) -> Option<&str> {
        self.offline_selected.as_deref()
    }
}

#[cfg(test)]
mod tests;
