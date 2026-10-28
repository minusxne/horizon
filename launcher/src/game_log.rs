//! The game's log as the console shows it, and a private copy of it: [`private`] takes out the
//! player's home folder, their user name and their offline names, for a log they pass on.
use std::path::Path;

/// Whether a console line matches the search: every word, in any case.
pub fn matches(line: &str, search: &str) -> bool {
    let line = line.to_lowercase();
    search
        .split_whitespace()
        .all(|w| line.contains(&w.to_lowercase()))
}

/// The names to take out of a log: the account names and offline names, the offline player
/// name kept in each game folder (`skyline-player.txt`), and the computer's user name.
pub fn names(homes: &[&Path]) -> Vec<String> {
    let accounts = crate::account::Accounts::load();
    let mut names: Vec<String> = accounts.offline;
    for home in homes {
        if let Ok(name) = std::fs::read_to_string(home.join("skyline-player.txt")) {
            names.push(name.trim().to_string());
        }
    }
    for var in ["USER", "USERNAME"] {
        if let Ok(user) = std::env::var(var) {
            names.push(user);
        }
    }
    if let Some(user) =
        dirs::home_dir().and_then(|h| h.file_name().map(|n| n.to_string_lossy().into_owned()))
    {
        names.push(user);
    }
    names.retain(|n| n.chars().count() >= 3);
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    names.dedup();
    names
}

/// The log without the home folder (`~` instead, with either slash) and without `names` (each
/// becomes `<player>`), as whole words in any case.
pub fn private(text: &str, home: Option<&Path>, names: &[String]) -> String {
    let mut out = text.to_string();
    if let Some(home) = home
        .map(|h| h.to_string_lossy().into_owned())
        .filter(|h| h.len() > 1)
    {
        out = out.replace(&home, "~");
        out = out.replace(&home.replace('\\', "/"), "~");
        out = out.replace(&home.replace('/', "\\"), "~");
    }
    for name in names {
        out = replace_word(&out, name, "<player>");
    }
    out
}

/// Replaces `word` (any case) where it isn't part of a longer name.
fn replace_word(text: &str, word: &str, with: &str) -> String {
    let lower = text.to_lowercase();
    let find = word.to_lowercase();
    if find.is_empty() || lower.len() != text.len() {
        // A case change that moves bytes (rare scripts): exact matches only.
        return text.replace(word, with);
    }
    let part = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(i) = lower[at..].find(&find).map(|i| i + at) {
        let end = i + find.len();
        if part(text[..i].chars().next_back()) || part(text[end..].chars().next()) {
            out.push_str(&text[at..end]);
        } else {
            out.push_str(&text[at..i]);
            out.push_str(with);
        }
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
#[path = "game_log/tests.rs"]
mod tests;
