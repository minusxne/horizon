use super::*;
use std::collections::BTreeMap;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "skyline-launcher-export-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn put(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// A gzipped level.dat whose player has a UUID.
fn level_dat() -> Vec<u8> {
    let mut player = BTreeMap::new();
    player.insert("UUID".into(), Tag::IntArray(vec![1, 2, 3, 4]));
    player.insert("XpLevel".into(), Tag::Int(7));
    let mut data = BTreeMap::new();
    data.insert("LevelName".into(), Tag::String("Base".into()));
    data.insert("Player".into(), Tag::Compound(player));
    let mut root = BTreeMap::new();
    root.insert("Data".into(), Tag::Compound(data));
    let raw = nbt::write_named(
        &nbt::NamedTag {
            name: "".into(),
            tag: Tag::Compound(root),
        },
        Limits::default(),
    )
    .unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw).unwrap();
    gz.finish().unwrap()
}

/// A game folder with settings, packs, a mod, two worlds, and what never goes out.
fn home(name: &str) -> PathBuf {
    let home = scratch(name);
    put(&home.join("options.txt"), b"lang:fr_ch\n");
    put(&home.join("skyline-graphics.txt"), b"preset:high\n");
    put(&home.join("servers.dat"), b"\x0a\x00\x00\x00");
    put(&home.join("resourcepacks/Glass.zip"), b"glass pack");
    put(&home.join("resourcepacks/Own.zip"), b"own pack");
    put(
        &home.join("skyline-mods/coords/horizon-mod.toml"),
        b"id = \"coords\"\n",
    );
    put(&home.join("saves/Base/level.dat"), &level_dat());
    put(&home.join("saves/Base/region/r.0.0.mca"), b"region");
    put(&home.join("saves/Base/playerdata/0001.dat"), b"me");
    put(&home.join("saves/Base/stats/0001.json"), b"{}");
    put(&home.join("saves/Base/session.lock"), b"");
    put(&home.join("saves/Other/level.dat"), &level_dat());
    for private in [
        "logs/latest.log",
        "screenshots/a.png",
        "backups/b.zip",
        "mod-browser/checks-1/x/check.json",
        "skyline-player.txt",
        "session.json",
        "usercache.json",
        "resourcepacks/.DS_Store",
    ] {
        put(&home.join(private), b"private");
    }
    home
}

fn names(zip: &Path) -> Vec<String> {
    let zip = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    let mut names: Vec<String> = zip.file_names().map(str::to_string).collect();
    names.sort();
    names
}

fn read(zip: &Path, name: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
    let mut out = Vec::new();
    zip.by_name(name).unwrap().read_to_end(&mut out).unwrap();
    out
}

#[test]
fn a_zip_has_the_instance_and_the_picked_world_and_nothing_private() {
    let home = home("zip");
    let out = home.join("out/Hardcore.zip");
    let done = export(&home, "Hardcore", Format::Zip, &["Base".into()], &[], &out).unwrap();
    assert_eq!((done.copied, done.linked), (8, 0));
    // Settings, packs, the mod and the picked world, without its per-player files; no logs,
    // screenshots, backups, caches, account files or the other world.
    assert_eq!(
        names(&out),
        [
            "options.txt",
            "resourcepacks/Glass.zip",
            "resourcepacks/Own.zip",
            "saves/Base/level.dat",
            "saves/Base/region/r.0.0.mca",
            "servers.dat",
            "skyline-graphics.txt",
            "skyline-mods/coords/horizon-mod.toml",
        ]
    );
    // The world's level.dat keeps the player, without the UUID.
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(&read(&out, "saves/Base/level.dat")[..])
        .read_to_end(&mut raw)
        .unwrap();
    let (root, _) = nbt::read_named(&raw, Limits::default()).unwrap();
    let Some(Tag::Compound(data)) = root.tag.get("Data") else {
        panic!("no Data")
    };
    let Some(Tag::Compound(player)) = data.get(&"Player".into()) else {
        panic!("no Player")
    };
    assert!(player.get(&"UUID".into()).is_none());
    assert_eq!(player.get(&"XpLevel".into()), Some(&Tag::Int(7)));
    assert!(!out.with_extension("part").exists());

    assert!(export(&home, "Hardcore", Format::Zip, &["../x".into()], &[], &out).is_err());
    let empty = scratch("empty");
    let refused = export(&empty, "New", Format::Zip, &[], &[], &empty.join("New.zip"));
    assert_eq!(
        refused.unwrap_err(),
        "New has nothing to share yet: no settings, packs, mods or worlds."
    );
    assert!(!empty.join("New.zip").exists());
    fs::remove_dir_all(&empty).unwrap();
    fs::remove_dir_all(&home).unwrap();
}
