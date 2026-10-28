use super::*;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("skyline-account-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn names_follow_the_games_rules() {
    assert!(valid_offline_name("Steve"));
    assert!(valid_offline_name("a"));
    assert!(valid_offline_name("x-Ray.9"));
    assert!(valid_offline_name("sixteen_chars_ok"));
    assert!(!valid_offline_name(""));
    assert!(!valid_offline_name("seventeen_chars_x"));
    assert!(!valid_offline_name("two words"));
    assert!(!valid_offline_name("Ștefan"));
    assert!(name_problem("Steve").is_none());
    assert!(name_problem(" ").is_some());
}

#[test]
fn an_offline_name_plays_without_any_account() {
    let mut a = Accounts::default();
    assert_eq!(a.player(), Player::GameName);
    a.play_as(Player::Offline("Alex".into())).unwrap();
    assert_eq!(a.player(), Player::Offline("Alex".into()));
    assert_eq!(a.name_arg(), Some("Alex"));
    // Kept once, in its first spelling.
    a.play_as(Player::Offline("alex".into())).unwrap();
    assert_eq!(a.offline, ["Alex"]);
    assert_eq!(a.name_arg(), Some("Alex"));
    assert!(a.play_as(Player::Offline("no spaces".into())).is_err());
    a.play_as(Player::GameName).unwrap();
    assert_eq!(a.name_arg(), None);
}

#[test]
fn removing_the_playing_name_falls_back_to_the_games() {
    let mut a = Accounts::default();
    a.play_as(Player::Offline("Alex".into())).unwrap();
    a.remove_offline("ALEX").unwrap();
    assert!(a.offline.is_empty());
    assert_eq!(a.player(), Player::GameName);
    assert!(a.remove_offline("Alex").is_err());
}

#[test]
fn old_files_with_microsoft_accounts_still_load() {
    let old = r#"{"accounts":[{"name":"x","uuid":"1"}],"selected":"1","offline":["Alex"],"offline_selected":"Alex","looks":{}}"#;
    let a: Accounts = serde_json::from_str(old).unwrap();
    assert_eq!(a.offline, ["Alex"]);
    assert_eq!(a.name_arg(), Some("Alex"));
}

#[test]
fn the_game_folder_keeps_the_name() {
    let home = scratch("game-name");
    assert_eq!(game_name(&home), None);
    std::fs::write(home.join(PLAYER_FILE), "name:Player12345\nother:1\n").unwrap();
    assert_eq!(game_name(&home).as_deref(), Some("Player12345"));
    write_game_name(&home, "Alex").unwrap();
    assert_eq!(game_name(&home).as_deref(), Some("Alex"));
    let text = std::fs::read_to_string(home.join(PLAYER_FILE)).unwrap();
    assert_eq!(text, "name:Alex\nother:1\n");
    assert!(write_game_name(&home, "bad name").is_err());
    let _ = std::fs::remove_dir_all(&home);
}
