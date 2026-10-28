use super::*;

#[test]
fn search_matches_every_word_in_any_case() {
    assert!(matches(
        "[12:00:01] [Render thread/WARN]: Missing texture",
        "warn texture"
    ));
    assert!(!matches("[12:00:01] [Render thread/INFO]: Loaded", "warn"));
    assert!(matches("anything", "  "));
}

#[test]
fn the_shared_log_has_no_home_folder_or_names() {
    let log = "Loading /Users/ludovicmarie/Library/Application Support/horizon/options.txt\n\
               C:\\Users\\ludovicmarie\\AppData\\horizon too\n\
               Ludo47 joined the game\n\
               <ludo47> hi, Ludo470 is someone else\n\
               user LUDOVICMARIE on ludovicmarie-mac\n";
    let names = vec!["ludovicmarie".to_string(), "Ludo47".to_string()];
    let out = private(log, Some(Path::new("/Users/ludovicmarie")), &names);
    assert_eq!(
        out,
        "Loading ~/Library/Application Support/horizon/options.txt\n\
         C:~\\AppData\\horizon too\n\
         <player> joined the game\n\
         <<player>> hi, Ludo470 is someone else\n\
         user <player> on <player>-mac\n"
    );
}
