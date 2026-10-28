use super::*;

const MANIFEST_TEXT: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
description = "Red gems"
api = "0.2"
side = "both"
capabilities = ["registries"]
"#;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "skyline-launcher-mods-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A content-only mod as a `.hmod` and as a folder (with another id).
fn sources(root: &Path) -> (PathBuf, PathBuf) {
    let hmod = root.join("downloads/ruby.hmod");
    fs::create_dir_all(hmod.parent().unwrap()).unwrap();
    fs::write(
        &hmod,
        horizon_mods::Package::write_files(MANIFEST_TEXT, None, &[]).unwrap(),
    )
    .unwrap();
    let folder = root.join("work/emerald");
    fs::create_dir_all(folder.join("assets/emerald")).unwrap();
    fs::write(
        folder.join(MANIFEST),
        MANIFEST_TEXT
            .replace("\"ruby\"", "\"emerald\"")
            .replace("Ruby", "Emerald"),
    )
    .unwrap();
    fs::write(folder.join("assets/emerald/note.txt"), "x").unwrap();
    (hmod, folder)
}

#[test]
fn mods_are_added_as_files_or_folders_and_start_off() {
    let root = scratch("add");
    let home = root.join("home");
    let (hmod, folder) = sources(&root);
    assert!(list(&home).is_empty());
    let ruby = add(&home, &hmod).unwrap();
    assert_eq!(ruby.id.as_deref(), Some("ruby"));
    assert_eq!(ruby.title, "Ruby 0.1.0");
    assert_eq!(ruby.description, "Red gems");
    assert!(!ruby.enabled && !ruby.folder);
    let emerald = add(&home, &folder).unwrap();
    assert!(emerald.folder);
    assert!(home
        .join("skyline-mods/emerald/assets/emerald/note.txt")
        .is_file());
    // The same file again gets a free name; the source stays.
    let again = add(&home, &hmod).unwrap();
    assert_eq!(again.file, "ruby (2).hmod");
    assert!(hmod.is_file());
    let files: Vec<String> = list(&home).into_iter().map(|m| m.file).collect();
    assert_eq!(files, ["emerald", "ruby (2).hmod", "ruby.hmod"]);
    // Not a mod: refused, nothing copied.
    let other = root.join("notes.txt");
    fs::write(&other, "x").unwrap();
    assert!(add(&home, &other).is_err());
    assert!(!home.join("skyline-mods/notes.txt").exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn turning_a_mod_on_writes_the_games_list_and_keeps_its_keys() {
    let root = scratch("toggle");
    let home = root.join("home");
    let (hmod, folder) = sources(&root);
    add(&home, &hmod).unwrap();
    add(&home, &folder).unwrap();
    fs::write(home.join(FILE), "key.ruby.dig:key.keyboard.g\n").unwrap();
    let ruby = find(&home, "ruby").unwrap();
    set_enabled(&home, &ruby, true).unwrap();
    let emerald = find(&home, "Emerald 0.1.0").unwrap();
    set_enabled(&home, &emerald, true).unwrap();
    let text = fs::read_to_string(home.join(FILE)).unwrap();
    assert!(text.contains("mod.ruby:true\n"), "{text}");
    assert!(text.contains("mod.emerald:true\n"), "{text}");
    assert!(text.contains("key.ruby.dig:key.keyboard.g\n"), "{text}");
    assert!(list(&home).iter().all(|m| m.enabled));
    set_enabled(&home, &find(&home, "ruby.hmod").unwrap(), false).unwrap();
    let text = fs::read_to_string(home.join(FILE)).unwrap();
    assert!(!text.contains("mod.ruby"), "{text}");
    assert!(text.contains("key.ruby.dig"), "{text}");
    assert!(!find(&home, "ruby").unwrap().enabled);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_broken_mod_is_listed_with_its_problem_and_cannot_be_turned_on() {
    let root = scratch("broken");
    let home = root.join("home");
    fs::create_dir_all(home.join(DIR)).unwrap();
    fs::write(home.join(DIR).join("broken.hmod"), "not a zip").unwrap();
    let all = list(&home);
    assert_eq!(all.len(), 1);
    assert!(all[0].problem.is_some());
    assert_eq!(all[0].id, None);
    assert!(set_enabled(&home, &all[0], true).is_err());
    assert!(!home.join(FILE).exists());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn removing_a_mod_turns_it_off_and_moves_it_away() {
    let root = scratch("remove");
    let home = root.join("home");
    let (hmod, _) = sources(&root);
    let ruby = add(&home, &hmod).unwrap();
    set_enabled(&home, &ruby, true).unwrap();
    let ruby = find(&home, "ruby").unwrap();
    // As remove() does, into a test trash folder instead of the system's.
    set_enabled(&home, &ruby, false).unwrap();
    crate::desktop_seams::to_folder(&ruby.path, &root.join("trash")).unwrap();
    assert!(list(&home).is_empty());
    assert!(!fs::read_to_string(home.join(FILE))
        .unwrap()
        .contains("ruby"));
    assert!(fs::read_dir(root.join("trash")).unwrap().count() == 1);
    fs::remove_dir_all(&root).unwrap();
}
