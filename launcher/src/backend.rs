//! Everything that is not UI: installed versions, resource preparation and the game's processes.
//! The GUI and the CLI both go through this module. Nothing here reaches the network. Functions
//! here block; the GUI runs them on threads and reads their messages.
use crate::sources;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// This launcher's version: a build can set `SKYLINE_LAUNCHER_VERSION`; other builds say
/// Cargo.toml's.
pub const VERSION: &str = version_or(option_env!("SKYLINE_LAUNCHER_VERSION"));
/// Launcher releases are tagged `launcher@X.Y.Z` from 0.2 on (docs/versioning.md), and Skyline's
/// own `vX.Y.Z`. Before 0.2 they were `launcher-vX.Y.Z` and `vYYYY.MM.DD[.N]`: every 0.2 tag is
/// newer than every one of those ([`release_order`]).
pub const LAUNCHER_TAG_PREFIX: &str = "launcher@";
/// The launcher tags from before 0.2 (`launcher-v0.16.0`).
pub const OLD_LAUNCHER_TAG_PREFIX: &str = "launcher-v";

const fn version_or(injected: Option<&'static str>) -> &'static str {
    match injected {
        Some(v) if !v.is_empty() => v,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

const LAUNCHER_BINARY: &str = if cfg!(windows) {
    "skyline-launcher.exe"
} else {
    "skyline-launcher"
};

pub(crate) const CLIENT: &str = if cfg!(windows) {
    "skyline.exe"
} else {
    "skyline"
};

/// The launcher version in a `launcher@X.Y.Z` (or `launcher-vX.Y.Z`) tag, or None for any other tag.
pub fn launcher_version(tag: &str) -> Option<&str> {
    tag.strip_prefix(LAUNCHER_TAG_PREFIX)
        .or_else(|| tag.strip_prefix(OLD_LAUNCHER_TAG_PREFIX))
        .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
}

/// Where a tag or version stands among Skyline's releases (docs/versioning.md): the tags from
/// before 0.2 (`vYYYY.MM.DD[.N]`, `launcher-vX.Y.Z`) and bare versions first, compared number by
/// number with missing parts as zero and anything after a `-` ignored, as launchers always did;
/// then the 0.2 tags (`vX.Y.Z`, `launcher@X.Y.Z`) in semver's order, a pre-release
/// (`v0.3.0-rc.1`) before its release.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReleaseOrder {
    /// Numbers without trailing zeros, so 1.2 and 1.2.0 are equal.
    Before02(Vec<u64>),
    Semver([u64; 3], Prerelease),
}

/// (is a release, the pre-release's identifiers): a release follows every pre-release of its
/// version; identifiers compare one by one, numbers before words, a shorter list first.
pub type Prerelease = (bool, Vec<Identifier>);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Identifier {
    Number(u64),
    Word(String),
}

/// Semver's majors stay below this; a date tag's first number is a year.
const FIRST_YEAR: u64 = 1000;

pub fn release_order(tag: &str) -> ReleaseOrder {
    let semver = tag
        .strip_prefix(LAUNCHER_TAG_PREFIX)
        .or_else(|| tag.strip_prefix('v'))
        .and_then(semver_order);
    semver.unwrap_or_else(|| {
        let v = tag
            .strip_prefix(OLD_LAUNCHER_TAG_PREFIX)
            .or_else(|| tag.strip_prefix(LAUNCHER_TAG_PREFIX))
            .unwrap_or(tag);
        let v = v.strip_prefix('v').unwrap_or(v);
        let mut parts: Vec<u64> = v
            .split('-')
            .next()
            .unwrap_or("")
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        while parts.last() == Some(&0) {
            parts.pop();
        }
        ReleaseOrder::Before02(parts)
    })
}

/// `MAJOR.MINOR.PATCH[-PRE]` with a major below [`FIRST_YEAR`], or None.
fn semver_order(version: &str) -> Option<ReleaseOrder> {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let numbers: Vec<u64> = core
        .split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<_>>()?;
    let &[major, minor, patch] = numbers.as_slice() else {
        return None;
    };
    if major >= FIRST_YEAR {
        return None;
    }
    let pre = match pre {
        None => (true, Vec::new()),
        Some(pre) => (
            false,
            pre.split('.')
                .map(|id| {
                    (!id.is_empty()).then(|| {
                        id.parse()
                            .map_or_else(|_| Identifier::Word(id.to_string()), Identifier::Number)
                    })
                })
                .collect::<Option<_>>()?,
        ),
    };
    Some(ReleaseOrder::Semver([major, minor, patch], pre))
}

/// Compares two tags or versions by [`release_order`].
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    release_order(a).cmp(&release_order(b))
}

/// The launcher's data folder; `SKYLINE_LAUNCHER_HOME` overrides it (for tests and scripts).
pub fn data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("SKYLINE_LAUNCHER_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    let data = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    horizon_core::rebrand::migrate_data_dirs(&data);
    data.join("skyline-launcher")
}

fn versions_dir() -> PathBuf {
    data_dir().join("versions")
}

pub fn version_dir(tag: &str) -> PathBuf {
    versions_dir().join(tag)
}

const COMPLETE: &str = ".installed";

/// The mod API level of the game players run: the selected version's, else the newest installed
/// one's. None with no version installed: the launcher doesn't know yet which game will run mods.
pub fn game_mod_api_level() -> Option<u32> {
    let installed = installed();
    let tag = Settings::load()
        .selected
        .filter(|s| installed.contains(s))
        .or_else(|| installed.into_iter().next())?;
    Some(mod_api_level(&tag))
}

/// A game version's mod API level, read once from the mark its program carries
/// (`horizon_mods::api_level::MARK`). A program from before the mark serves level 0, the
/// contract of Skyline 0.2.67 and 0.2.68.
pub fn mod_api_level(tag: &str) -> u32 {
    static LEVELS: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());
    let mut levels = LEVELS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&(_, level)) = levels.iter().find(|(t, _)| t == tag) {
        return level;
    }
    let level = program_mod_api_level(&version_dir(tag).join(CLIENT));
    levels.push((tag.to_owned(), level));
    level
}

/// The mod API level in the game program at `path`: its mark's, else 0.
fn program_mod_api_level(path: &Path) -> u32 {
    let program = fs::read(path).unwrap_or_default();
    horizon_mods::api_level::read_mark(&program).unwrap_or(0)
}

/// Installed versions, newest tag first.
pub fn installed() -> Vec<String> {
    installed_in(&versions_dir())
}

fn installed_in(versions: &Path) -> Vec<String> {
    let mut tags: Vec<String> = fs::read_dir(versions)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join(COMPLETE).exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    tags.sort_by(|a, b| compare_versions(b, a));
    tags
}

pub fn remove(tag: &str) -> Result<(), String> {
    let dir = version_dir(tag);
    // Windows holds a file for a moment after the game, a scan or the launcher's own reader lets
    // go of it, and remove_dir_all stops at the first one. Retry for a second; a folder that is
    // already gone is removed.
    for attempt in 1.. {
        match fs::remove_dir_all(&dir) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) if attempt == 5 => return Err(e.to_string()),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(250)),
        }
    }
    Ok(())
}

/// The Minecraft version a Skyline build targets, from its research-lock.json.
pub fn game_version(tag: &str) -> String {
    game_version_at(&version_dir(tag))
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn game_version_at(dir: &Path) -> String {
    read_json(&dir.join("research-lock.json"))
        .and_then(|v| v["minecraft"]["version"].as_str().map(String::from))
        .unwrap_or_else(|| horizon_core::VANILLA_VERSION.into())
}

pub fn resources(tag: &str) -> PathBuf {
    resources_at(&version_dir(tag))
}

fn resources_at(dir: &Path) -> PathBuf {
    horizon_core::rebrand::migrate_resources(dir);
    dir.join(".local/skyline").join(game_version_at(dir))
}

const RESOURCES_MANIFEST: &str = "skyline-resources.json";

pub fn is_prepared(tag: &str) -> bool {
    resources(tag).join(RESOURCES_MANIFEST).exists()
}

/// Whether a version's resources hold Minecraft's sounds, which `sounds` copies from the player's own
/// install beside the preparation. A version prepared while that copy failed (Mojang's republished 26.3
/// asset index, #1076), or one that reused such a version's resources, plays no sound at all (#1035).
pub fn has_sounds(tag: &str) -> bool {
    sounds_in(&resources(tag))
}

fn sounds_in(resources: &Path) -> bool {
    // What the game's sound registry reads (horizon-client sound_events.rs).
    resources.join("assets/minecraft/sounds.json").is_file()
}

/// What the Play page says of a version's resources.
pub fn resources_summary(prepared: bool, sounds: bool, version: &str) -> String {
    use crate::i18n::trf;
    match (prepared, sounds) {
        (false, _) => trf(
            "Not prepared. Skyline needs the files of your own Minecraft {version}.",
            &[("version", version)],
        ),
        (true, true) => trf("Prepared for Minecraft {version}", &[("version", version)]),
        (true, false) => trf(
            "Prepared for Minecraft {version}, without its sounds: the game will be silent. \
             Prepare again to copy them from your Minecraft install.",
            &[("version", version)],
        ),
    }
}

/// Whether the resources `from` prepared are the ones preparing `to` would make: both pin the same
/// Minecraft jars (research-lock.json) and their preparers make the same output. A preparer
/// declares what it makes with `OUTPUT_VERSION` (crates/horizon-prepare/src/lib.rs, whose sources
/// ship in the release folder), bumped by hand when the output changes and checked by CI
/// (tools/prepare_output.py), so a release that changes the preparer's code but not its output
/// keeps the resources. Preparers from before the number are known by their sources
/// (`LEGACY_OUTPUT_VERSIONS`); older ones still are compared file by file (`same_preparer`).
fn resources_compatible(from: &Path, to: &Path, manifest: &serde_json::Value) -> bool {
    let pins = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).map(|lock| {
            ["version", "client_sha1", "server_sha1"].map(|k| lock["minecraft"][k].clone())
        })
    };
    let (Some(a), Some(b)) = (pins(from), pins(to)) else {
        return false;
    };
    if a != b || a.iter().any(|v| !v.is_string()) {
        return false;
    }
    match (output_version_at(from), output_version_at(to)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => same_preparer(from, to, manifest),
        _ => false,
    }
}

/// The `OUTPUT_VERSION` of a version folder's preparer: the constant in its sources, or for a
/// preparer from before it, the number `LEGACY_OUTPUT_VERSIONS` gives its fingerprint. None for
/// an unknown older preparer.
fn output_version_at(dir: &Path) -> Option<u32> {
    output_version_in(dir, &LEGACY_OUTPUT_VERSIONS)
}

fn output_version_in(dir: &Path, legacy: &[(&str, u32)]) -> Option<u32> {
    let crate_dir = dir.join(PREPARER_CRATE);
    let lib = fs::read_to_string(crate_dir.join("src/lib.rs")).ok()?;
    let declared = lib.lines().find_map(|line| {
        line.strip_prefix("pub const OUTPUT_VERSION: u32 = ")?
            .strip_suffix(';')?
            .parse()
            .ok()
    });
    if declared.is_some() {
        return declared;
    }
    let fingerprint = legacy_fingerprint(dir)?;
    legacy
        .iter()
        .find(|(known, _)| *known == fingerprint)
        .map(|&(_, version)| version)
}

/// Preparers from before `OUTPUT_VERSION`, by `legacy_fingerprint`, whose output
/// tools/prepare_output.py measured: what they prepare is version 1's.
const LEGACY_OUTPUT_VERSIONS: [(&str, u32); 2] = [
    // v0.2.52 to v0.2.64
    (
        "54218ade1c058b1f15ea4094bf8c1a7893adb37b1ed5cc803ddae8e950c31704",
        1,
    ),
    // v0.2.49 to v0.2.51
    (
        "da9ffe7558129c38b44abf43289e81946fe9f2512e5e07272eeae0cf0b4f1254",
        1,
    ),
];

/// The vanilla exporters those preparers ran (the `*_exporters_sha256` of their manifests), in
/// mods/horizon/src/test/java/dev/horizonlab/parity.
const LEGACY_EXPORTERS: [&str; 18] = [
    "ParityData.java",
    "VanillaBlockBehaviourExporter.java",
    "VanillaBlockLandingOracle.java",
    "VanillaBlockStateBindingOracle.java",
    "VanillaCollisionOracle.java",
    "VanillaCommandParseOracle.java",
    "VanillaCreativeTabsExporter.java",
    "VanillaCuboidModelOracle.java",
    "VanillaDamageSourceOracle.java",
    "VanillaEntityAttributeOracle.java",
    "VanillaEntityModelExporter.java",
    "VanillaEntityTypesExporter.java",
    "VanillaFaceVisibilityOracle.java",
    "VanillaGameplayFactsExporter.java",
    "VanillaHeightmapOracle.java",
    "VanillaMobDataExporter.java",
    "VanillaMovementRegistryOracle.java",
    "VanillaPlayerHealthOracle.java",
];

/// A version folder's preparer as the launchers before `OUTPUT_VERSION` compared it: the digest
/// of its sources (`horizon_prepare::sources_sha256`) and of the exporters it ran, line endings
/// as checked out ignored.
fn legacy_fingerprint(dir: &Path) -> Option<String> {
    use sha2::Digest;
    let sources = horizon_prepare::sources_sha256(&dir.join(PREPARER_CRATE))?;
    let mut hash = sha2::Sha256::new();
    hash.update(sources.as_bytes());
    for name in LEGACY_EXPORTERS {
        let path = format!("mods/horizon/src/test/java/dev/horizonlab/parity/{name}");
        let data: Vec<u8> = fs::read(dir.join(&path))
            .ok()?
            .into_iter()
            .filter(|b| *b != b'\r')
            .collect();
        hash.update(format!("{path}\0{}\0", data.len()));
        hash.update(&data);
    }
    Some(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// How launchers before `OUTPUT_VERSION` compared two preparers, for versions older than the
/// ones `LEGACY_OUTPUT_VERSIONS` knows. The manifest records the hashes of the vanilla exporters
/// the resources were made with (mods/horizon/src/test/java/.../parity), and the preparer writes
/// every file and the manifest's `schema`, so both need the same preparer and the same exporter
/// sources. The preparer is crates/horizon-prepare and tools/prepare_native_client.py (the
/// Python preparer of older versions, a wrapper of the binary since); a file only one of the two
/// versions has is a different preparer. The helpers the Python preparer imported (lab.py and
/// the test_*_parity.py modules) change between releases in how they find and check the
/// player's files, which the output does not depend on; they are not compared.
fn same_preparer(from: &Path, to: &Path, manifest: &serde_json::Value) -> bool {
    let Some(fields) = manifest.as_object() else {
        return false;
    };
    let mut preparer = vec![PathBuf::from("tools/prepare_native_client.py")];
    for dir in [from, to] {
        preparer.extend(files_under(dir, Path::new(PREPARER_CRATE)));
    }
    let same_or_absent = |path: &Path| match (fs::read(from.join(path)), fs::read(to.join(path))) {
        (Ok(a), Ok(b)) => a == b,
        (Err(a), Err(b)) => {
            a.kind() == std::io::ErrorKind::NotFound && b.kind() == std::io::ErrorKind::NotFound
        }
        _ => false,
    };
    if !preparer.iter().all(|p| same_or_absent(p)) {
        return false;
    }
    let mut sources = vec![
        "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java".to_string(),
    ];
    for (_, value) in fields
        .iter()
        .filter(|(key, _)| key.ends_with("_exporters_sha256"))
    {
        let Some(files) = value.as_object() else {
            return false;
        };
        sources.extend(files.keys().cloned());
    }
    let same = |source: &str| {
        let path = Path::new(source);
        path.is_relative()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && matches!((fs::read(from.join(path)), fs::read(to.join(path))), (Ok(a), Ok(b)) if a == b)
    };
    sources.iter().all(|s| same(s))
}

/// Where a version folder keeps the preparer's sources (the release package copies them).
const PREPARER_CRATE: &str = "crates/horizon-prepare";

/// The files under `dir/relative`, as paths relative to `dir`.
fn files_under(dir: &Path, relative: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir.join(relative)) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = relative.join(entry.file_name());
        match entry.file_type() {
            Ok(t) if t.is_dir() => files.extend(files_under(dir, &path)),
            Ok(_) => files.push(path),
            Err(_) => {}
        }
    }
    files
}

pub(crate) fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            horizon_prepare::clone_or_copy(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// After an install: when `tag` is not prepared but another installed version made the same
/// resources, copies them (not hard links: preparing again rewrites files in place, which would
/// change both). The player then needs neither their Minecraft folder nor a new preparation to
/// play a release that only changed Skyline. Returns the version they came from.
fn reuse_prepared(versions: &Path, tag: &str) -> Option<String> {
    let target = versions.join(tag);
    let to = resources_at(&target);
    if to.join(RESOURCES_MANIFEST).exists() {
        return None;
    }
    let version = game_version_at(&target);
    for other in installed_in(versions).into_iter().filter(|t| t != tag) {
        let source = versions.join(&other);
        if game_version_at(&source) != version {
            continue;
        }
        let from = resources_at(&source);
        let Some(manifest) = read_json(&from.join(RESOURCES_MANIFEST)) else {
            continue;
        };
        if !resources_compatible(&source, &target, &manifest) {
            continue;
        }
        // Copied beside it and renamed when complete: an interrupted copy is never "prepared".
        // The name is this call's own, as the window's start and an install can both copy.
        static COPIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COPIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let partial = to.with_file_name(format!(".{version}.partial-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&partial);
        let copied = copy_dir(&from, &partial).and_then(|_| {
            if to.join(RESOURCES_MANIFEST).exists() {
                return Ok(false); // Prepared meanwhile, by another copy or the player.
            }
            let _ = fs::remove_dir_all(&to);
            fs::rename(&partial, &to).map(|_| true)
        });
        let _ = fs::remove_dir_all(&partial);
        match copied {
            Ok(true) => return Some(other),
            Ok(false) => return None,
            Err(_) => {}
        }
    }
    None
}

/// `reuse_prepared` for every installed version that is not prepared, oldest first, so a
/// version prepared this way can serve the next. Launchers before 0.1.4 installed new versions
/// without it; this catches those up. Returns each (version, where its resources came from).
/// Prepared versions also get the run-time reports (`complete_reports`) and the built-in packs,
/// panorama and sounds (`complete_assets`) they lack.
pub fn reuse_prepared_everywhere() -> Vec<(String, String)> {
    let versions = versions_dir();
    let reused = reuse_prepared_in(&versions);
    for tag in installed_in(&versions) {
        complete_assets(&tag);
    }
    reused
}

fn reuse_prepared_in(versions: &Path) -> Vec<(String, String)> {
    let mut tags = installed_in(versions);
    tags.reverse();
    let reused = tags
        .iter()
        .filter_map(|tag| reuse_prepared(versions, tag).map(|from| (tag.clone(), from)))
        .collect();
    for tag in &tags {
        complete_reports(versions, tag);
    }
    reused
}

/// The data generator reports the game reads at run time: the built-in registries' network ids
/// (without them no server can be joined or hosted) and the command tree.
const RUNTIME_REPORTS: [&str; 2] = ["registries.json", "commands.json"];

/// Puts the data generator reports the game reads at run time (`RUNTIME_REPORTS`) into a prepared version's
/// resources when they are missing. Preparers before they shipped there left them only in the
/// scratch output of the version that ran the data generator (`.local/datagen/out/reports`),
/// and resources copied from another version (`reuse_prepared`) came without them. They depend
/// only on the server jar, so the version's own scratch output or any installed version pinning
/// the same jar serves, and no new preparation is needed. Returns whether all are there.
fn complete_reports(versions: &Path, tag: &str) -> bool {
    let target = versions.join(tag);
    let to = resources_at(&target);
    if !to.join(RESOURCES_MANIFEST).exists() {
        return false;
    }
    let missing: Vec<_> = RUNTIME_REPORTS
        .iter()
        .filter(|name| !to.join(name).is_file())
        .collect();
    if missing.is_empty() {
        return true;
    }
    let server = |dir: &Path| {
        read_json(&dir.join("research-lock.json")).and_then(|lock| {
            let pins = ["version", "server_sha1"].map(|k| lock["minecraft"][k].clone());
            pins.iter().all(|v| v.is_string()).then_some(pins)
        })
    };
    let Some(pins) = server(&target) else {
        return false;
    };
    let reports = |dir: &Path| dir.join(".local/datagen/out/reports");
    let mut sources = vec![reports(&target)];
    for other in installed_in(versions).into_iter().filter(|t| t != tag) {
        let dir = versions.join(&other);
        if server(&dir).as_ref() == Some(&pins) {
            sources.push(resources_at(&dir));
            sources.push(reports(&dir));
        }
    }
    let mut complete = true;
    for name in missing {
        let Some(from) = sources.iter().map(|s| s.join(name)).find(|p| p.is_file()) else {
            complete = false;
            continue;
        };
        // Copied beside it and renamed: an interrupted copy never looks complete.
        let partial = to.join(format!(".{name}.partial-{}", std::process::id()));
        let copied = fs::copy(&from, &partial).and_then(|_| fs::rename(&partial, to.join(name)));
        if copied.is_err() {
            let _ = fs::remove_file(&partial);
            complete = false;
        }
    }
    complete
}

/// What a prepared version may lack that the launcher adds without preparing again, each copied
/// from the player's own Minecraft asset store as `skyline-prepare <kind>` does: what the
/// player misses without it, whether the resources lack it, and the copy.
type Completion = (
    &'static str,
    fn(&Path) -> bool,
    fn(&Path, Option<&Path>, horizon_prepare::Log) -> Result<PathBuf, String>,
);

const COMPLETIONS: [Completion; 5] = [
    // Preparers before #1538 left the built-in packs out.
    (
        "Programmer Art and High Contrast",
        lacks_packs,
        horizon_prepare::packs,
    ),
    // A preparation that could not read the asset store keeps the client jar's 1x1 grey
    // placeholders, its English alone, and no sounds (#1867: Prism Launcher installs prepared
    // since 0.2.48).
    (
        "The title panorama",
        lacks_panorama,
        horizon_prepare::panorama,
    ),
    ("The Unifont glyphs", lacks_unifont, horizon_prepare::fonts),
    ("The languages", lacks_languages, horizon_prepare::languages),
    (
        "The sounds",
        |resources| !sounds_in(resources),
        horizon_prepare::sounds,
    ),
];

fn lacks_unifont(resources: &Path) -> bool {
    !resources
        .join("assets/minecraft/font/unifont.zip")
        .is_file()
}

/// Whether `en_us`, the one language the client jar has, is the only one.
fn lacks_languages(resources: &Path) -> bool {
    fs::read_dir(resources.join("assets/minecraft/lang")).map_or(true, |dir| dir.count() <= 1)
}

fn lacks_packs(resources: &Path) -> bool {
    horizon_prepare::BUILT_IN_PACKS
        .iter()
        .any(|pack| !resources.join("assets").join(pack).is_file())
}

/// Whether the first panorama face is missing or the client jar's 1x1 placeholder (a PNG's
/// width is the big-endian word at byte 16, in the IHDR chunk).
fn lacks_panorama(resources: &Path) -> bool {
    let face = resources.join("assets/minecraft/textures/gui/title/background/panorama_0.png");
    fs::read(face).map_or(true, |png| {
        png.get(16..20).is_none_or(|width| width == [0, 0, 0, 1])
    })
}

/// Adds what a prepared version lacks of `COMPLETIONS`: a new version reuses the resources of the
/// one before (`reuse_prepared`), which may come from a preparer that left them out. Without them
/// in any Minecraft folder, one line each in the log, once per launcher run, and the version plays
/// as before. Nothing is prepared again.
pub fn complete_assets(tag: &str) {
    complete_assets_in(&versions_dir(), tag, minecraft_folders, &|line| {
        eprintln!("{line}")
    });
}

/// `folders`: where to look, asked only when the version lacks something.
fn complete_assets_in(
    versions: &Path,
    tag: &str,
    folders: impl Fn() -> Vec<PathBuf>,
    log: &dyn Fn(String),
) {
    // The versions tried this run. Held while copying: the start, an install and Play can
    // complete the same version at once, and the copies' temporary names are per process.
    static TRIED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let root = versions.join(tag);
    let resources = resources_at(&root);
    let lacking = || -> Vec<&Completion> {
        if !resources.join(RESOURCES_MANIFEST).exists() {
            return Vec::new();
        }
        COMPLETIONS
            .iter()
            .filter(|(_, lacks, _)| lacks(&resources))
            .collect()
    };
    if lacking().is_empty() {
        return;
    }
    let mut tried = TRIED.lock().unwrap_or_else(|e| e.into_inner());
    let lacking = lacking();
    if tried.contains(&root) || lacking.is_empty() {
        return;
    }
    tried.push(root.clone());
    let folders = folders();
    for (what, _, copy) in lacking {
        let mut first_error = None;
        let added = folders.iter().any(|folder| {
            let copied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                copy(&root, Some(folder), &|line| log(line.to_string()))
            }))
            .unwrap_or_else(|_| Err("the copy stopped unexpectedly".into()));
            copied.map_err(|e| first_error.get_or_insert(e)).is_ok()
        });
        if !added {
            log(format!(
                "{tag}: {what} not added: {}",
                first_error.unwrap_or_else(|| "no Minecraft folder found".into())
            ));
        }
    }
}

/// The Minecraft folders whose asset store can hold what a prepared version lacks: the one
/// chosen in the launcher, the official launcher's (or SKYLINE_MINECRAFT_DIR), the folder
/// other launchers' files are linked into and Skyline's own download, then every launcher's
/// default folder.
fn minecraft_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = Settings::load().minecraft_dir.into_iter().collect();
    folders.extend([
        horizon_prepare::artifacts::minecraft_dir(None),
        data_dir().join("linked-minecraft"),
        data_dir().join("minecraft"),
    ]);
    folders.extend(
        sources::Folders::current()
            .map(|f| sources::known_folders(sources::Os::current(), &f))
            .unwrap_or_default()
            .into_iter()
            .map(|(_, dir)| dir),
    );
    let mut found: Vec<PathBuf> = Vec::new();
    for folder in folders {
        if folder.is_dir() && !found.contains(&folder) {
            found.push(folder);
        }
    }
    found
}

/// The official launcher's game directory, else the first other launcher's found
/// (sources.rs); preparing looks in all of them anyway.
pub fn default_minecraft_dir() -> Option<PathBuf> {
    let folders = sources::Folders::current()?;
    sources::known_folders(sources::Os::current(), &folders)
        .into_iter()
        .map(|(_, dir)| dir)
        .find(|dir| dir.is_dir())
}

fn found(program: &str, arg: &str) -> bool {
    crate::process::command(program)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn python() -> Option<&'static str> {
    let candidates: &[&'static str] = if cfg!(windows) {
        &["py", "python"]
    } else {
        &["python3", "python"]
    };
    candidates.iter().copied().find(|p| found(p, "--version"))
}

/// How a version's resources are prepared.
pub enum Preparer {
    /// skyline-prepare, linked into the launcher: the version ships the same preparer sources.
    InProcess(horizon_prepare::Options),
    /// The version's own preparer: its skyline-prepare binary when its sources differ from the
    /// launcher's (a newer or older release), or the Python script of versions made before it.
    Process(Command),
}

/// The version's skyline-prepare binary, when it ships one.
fn preparer_binary(dir: &Path) -> Option<PathBuf> {
    let binary = dir.join(if cfg!(windows) {
        "skyline-prepare.exe"
    } else {
        "skyline-prepare"
    });
    binary.is_file().then_some(binary)
}

/// The Java home for this version: a runtime for precompiled exporters, a JDK for older
/// preparers or changed sources. Forward the selection to external preparers too.
/// Else try the official launcher's Java in `games` (Mojang's runtimes are whole JDKs). Nothing
/// is downloaded.
fn java(
    major: i64,
    compiler: bool,
    games: &[PathBuf],
    manifest: &Path,
    log: &dyn Fn(String),
) -> Result<PathBuf, String> {
    let find = if compiler {
        horizon_prepare::java::find
    } else {
        horizon_prepare::java::find_runtime
    };
    if let Ok(jdk) = find(major, &[]) {
        return Ok(jdk.home());
    }
    let mut runtimes: Vec<PathBuf> = Vec::new();
    for home in games
        .iter()
        .flat_map(|game| horizon_prepare::java::minecraft_runtimes(&game.join("runtime")))
    {
        if !runtimes.contains(&home) {
            runtimes.push(home);
        }
    }
    let missing = match find(major, &runtimes) {
        Ok(jdk) => return Ok(jdk.home()),
        Err(e) => e,
    };
    let _ = (manifest, log);
    Err(format!(
        "{missing}\n  • Skyline is offline and downloads no Java: install Java {major} (a JDK), or \
         let Prism Launcher or the official launcher install Minecraft's own Java."
    ))
}

/// Runs `cmd` with Java at `home`: JAVA_HOME names it and its `bin` leads the PATH, as the
/// versions' own preparers look for Java.
fn with_java(cmd: &mut Command, home: &Path) {
    let mut path = vec![home.join("bin")];
    path.extend(
        std::env::var_os("PATH")
            .iter()
            .flat_map(std::env::split_paths),
    );
    cmd.env("JAVA_HOME", home);
    if let Ok(path) = std::env::join_paths(path) {
        cmd.env("PATH", path);
    }
}

/// The Minecraft folder a preparation whose game files are imported already reads: the sounds,
/// the title panorama, the fonts and the languages come from its asset store. It is the one
/// `resolve` found, linked into the official layout for another launcher's files; without one, the
/// chosen folder, and the preparation goes without them, saying so. Before, such a preparation
/// read the chosen folder as it was, which a Prism Launcher or Modrinth App install does not
/// keep its assets in: the game was silent and its title screen grey.
fn assets_folder(
    resolved: Result<sources::Resolved, String>,
    chosen: &Path,
    version: &str,
    log: &dyn Fn(String),
) -> PathBuf {
    match resolved {
        Ok(sources::Resolved::Folder(dir)) => dir,
        Ok(sources::Resolved::NotFound(_)) => {
            log(format!(
                "Minecraft {version}'s assets were not found in any launcher folder: the game \
                 will be silent and the title screen plain"
            ));
            chosen.to_path_buf()
        }
        Err(e) => {
            log(format!("Minecraft {version}'s assets: {e}"));
            chosen.to_path_buf()
        }
    }
}

/// Where a Minecraft server jar may already be on this computer, the likeliest first.
fn server_jar_candidates(version: &str, minecraft: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::env::var_os("SKYLINE_SERVER_JAR")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    let in_version = minecraft.join("versions").join(version);
    found.push(in_version.join("server.jar"));
    found.push(in_version.join(format!("{version}-server.jar")));
    for dir in installed()
        .iter()
        .map(|t| version_dir(t).join(".local/server-jars"))
    {
        found.push(dir.join(format!("server-{version}.jar")));
        found.push(dir.join(format!("{version}.jar")));
    }
    if let Some(downloads) = dirs::download_dir() {
        found.push(downloads.join(format!("minecraft_server.{version}.jar")));
        found.push(downloads.join(format!("server-{version}.jar")));
        found.push(downloads.join("server.jar"));
    }
    found
}

/// Keeps a server jar the player picked as the one preparing `version` uses.
pub fn use_server_jar(version: &str, picked: &Path) -> Result<PathBuf, String> {
    let is_jar = picked
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("jar"));
    if !is_jar || !picked.is_file() {
        return Err(format!("{} is not a .jar file.", picked.display()));
    }
    let jar = data_dir()
        .join("server-jars")
        .join(format!("{version}.jar"));
    fs::create_dir_all(jar.parent().unwrap()).map_err(|e| e.to_string())?;
    fs::copy(picked, &jar).map_err(|e| e.to_string())?;
    Ok(jar)
}

/// Checks what the preparation needs and picks how to run it. PLAY.txt describes the same
/// steps: the player's own launcher install, the server jar from that version's own json, and
/// Java (and Python, for versions before skyline-prepare). Also returns the Minecraft folder
/// to prepare from: `minecraft`, or the files another launcher keeps, linked into the official
/// launcher's layout (sources.rs).
pub fn preparer(
    tag: &str,
    minecraft: &Path,
    log: &dyn Fn(String),
) -> Result<(Preparer, PathBuf), String> {
    let version = game_version(tag);
    let dir = version_dir(tag);
    let reused = horizon_prepare::artifacts::reuse_imported(&dir, &|line| log(line.to_string()))?;
    let chosen = minecraft.to_path_buf();
    // Resolved even when the game files are reused: the assets still come from the install.
    let minecraft = {
        let pins = read_json(&dir.join("research-lock.json"))
            .and_then(|lock| sources::Pins::read(&lock))
            .ok_or("The version's research-lock.json doesn't pin the Minecraft files")?;
        let own = data_dir().join("minecraft");
        let mut known = vec![("Skyline's own download", own.clone())];
        known.extend(
            sources::Folders::current()
                .map(|f| sources::known_folders(sources::Os::current(), &f))
                .unwrap_or_default(),
        );
        let fetch = offline_fetch;
        let linked = data_dir().join("linked-minecraft");
        let resolved = sources::resolve(minecraft, &pins, &known, &linked, &fetch, log);
        if reused {
            assets_folder(resolved, &chosen, &version, log)
        } else {
            match resolved? {
                sources::Resolved::Folder(dir) => dir,
                sources::Resolved::NotFound(checked) => {
                    return Err(sources::not_found(&version, &checked, minecraft));
                }
            }
        }
    };
    let manifest = minecraft
        .join("versions")
        .join(&version)
        .join(format!("{version}.json"));
    let binary = preparer_binary(&dir);
    let python = match binary {
        Some(_) => None,
        None => Some(python().ok_or(
            "This version prepares with Python: Python 3.10 or newer is needed. Install it, or pick a newer version.",
        )?),
    };
    // The Java the version pins, found as skyline-prepare finds it, so a missing one is
    // reported (or downloaded) before the server jar is downloaded. Versions from before
    // java_major ran Java 25.
    let major = read_json(&dir.join("research-lock.json"))
        .and_then(|lock| lock["minecraft"]["java_major"].as_i64())
        .unwrap_or(25);
    let mut games = vec![chosen, minecraft.clone(), data_dir().join("minecraft")];
    games.extend(
        sources::Folders::current()
            .map(|f| sources::known_folders(sources::Os::current(), &f))
            .unwrap_or_default()
            .into_iter()
            .map(|(_, dir)| dir),
    );
    let java_home = java(
        major,
        !horizon_prepare::exporters::available(&dir),
        &games,
        &manifest,
        log,
    )?;
    let jar = data_dir()
        .join("server-jars")
        .join(format!("{version}.jar"));
    if !reused && !jar.exists() {
        // Skyline downloads nothing: the server jar comes from this computer.
        let found = server_jar_candidates(&version, &minecraft)
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| {
                format!(
                    "The Minecraft {version} server jar is needed once to prepare. Put it at {} (Prism and the official launcher download it as server.jar), then press Prepare again.",
                    jar.display()
                )
            })?;
        log(format!(
            "Using the {version} server jar at {}",
            found.display()
        ));
        fs::create_dir_all(jar.parent().unwrap()).map_err(|e| e.to_string())?;
        let partial = jar.with_extension("partial");
        fs::copy(&found, &partial)
            .and_then(|_| fs::rename(&partial, &jar))
            .map_err(|e| e.to_string())?;
    }
    if let Some(binary) = binary {
        if horizon_prepare::sources_sha256(&dir.join(PREPARER_CRATE)).as_deref()
            == Some(horizon_prepare::SOURCES_SHA256)
        {
            let options = horizon_prepare::Options {
                root: dir,
                import: !reused,
                minecraft_dir: Some(minecraft.clone()),
                server_jar: (!reused).then_some(jar),
                java_home: Some(java_home.clone()),
            };
            return Ok((Preparer::InProcess(options), minecraft));
        }
        let mut cmd = Command::new(binary);
        with_java(&mut cmd, &java_home);
        cmd.current_dir(&dir).arg("--root").arg(&dir);
        if !reused {
            cmd.args(["--prepare", "--server-jar"]).arg(&jar);
        }
        cmd.arg("--minecraft-dir").arg(&minecraft);
        return Ok((Preparer::Process(cmd), minecraft));
    }
    let mut cmd = Command::new(python.unwrap());
    with_java(&mut cmd, &java_home);
    cmd.current_dir(&dir)
        .args(["-u", "tools/prepare_native_client.py"]);
    if !reused {
        cmd.args(["--prepare", "--server-jar"]).arg(&jar);
    }
    cmd.arg("--minecraft-dir").arg(&minecraft);
    Ok((Preparer::Process(cmd), minecraft))
}

/// What preparing does where it would once have downloaded a file: it says where to get it.
fn offline_fetch(url: &str) -> Result<Vec<u8>, String> {
    let name = url.rsplit('/').next().unwrap_or(url);
    Err(format!(
        "is not on this computer, and Skyline downloads nothing: play this Minecraft version once \
         in the official launcher so it keeps {name}, or pick its folder"
    ))
}

/// Copies the pinned version's sounds from the player's own launcher install into the version's
/// resources (`skyline-prepare sounds`), each line to `log`. Sound is optional, so a launcher
/// install without them is a line in the log, never a failed preparation. The version's own
/// preparer does it when it is not the launcher's and knows how; versions from before it read
/// the sounds where the launcher's preparer puts them.
pub fn sounds(tag: &str, minecraft: &Path, log: &dyn Fn(String)) {
    let dir = version_dir(tag);
    let own = horizon_prepare::sources_sha256(&dir.join(PREPARER_CRATE)).as_deref()
        == Some(horizon_prepare::SOURCES_SHA256);
    let binary = preparer_binary(&dir).filter(|binary| {
        !own && crate::process::command(binary)
            .arg("--help")
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("skyline-prepare sounds"))
    });
    log("Copying the sounds from your Minecraft install".into());
    let result = match binary {
        Some(binary) => crate::process::command(binary)
            .current_dir(&dir)
            .arg("sounds")
            .arg("--root")
            .arg(&dir)
            .arg("--minecraft-dir")
            .arg(minecraft)
            .output()
            .map_err(|e| e.to_string())
            .and_then(|o| {
                let text = |b: &[u8]| String::from_utf8_lossy(b).trim().to_string();
                for line in text(&o.stdout).lines() {
                    log(line.to_string());
                }
                match o.status.success() {
                    true => Ok(()),
                    false => Err(text(&o.stderr).trim_start_matches("error: ").to_string()),
                }
            }),
        None => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            horizon_prepare::sounds(&dir, Some(minecraft), &|line| log(line.to_string()))
        }))
        .unwrap_or_else(|_| Err("The sound copy stopped unexpectedly".into()))
        .map(|_| ()),
    };
    if let Err(e) = result {
        log(format!("No sound (the game will be silent): {e}"));
    }
}

/// Runs `Preparer::InProcess` on this thread, each progress line to `log`. A panic is an
/// error like any other, so the window never waits on a preparation that died.
pub fn prepare_in_process(
    options: &horizon_prepare::Options,
    log: &dyn Fn(String),
) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        horizon_prepare::prepare(options, &|line| log(line.to_string()))
    }))
    .unwrap_or_else(|_| Err("The preparation stopped unexpectedly".into()))
    .map(|_| ())
}

/// Where the game keeps saves and options.txt: `SKYLINE_HOME`, else `horizon` in the platform's
/// data folder (~/.local/share/skyline on Linux, as the release folders' horizon.sh uses). It is
/// never inside a version folder, so installing, updating or removing a version keeps the saves.
pub fn game_home() -> PathBuf {
    if let Some(home) = std::env::var_os("SKYLINE_HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(home);
    }
    let data = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    horizon_core::rebrand::migrate_data_dirs(&data);
    data.join("skyline")
}

// ----- Logs and crash reports ---------------------------------------------------------------------

/// The game's logs (`latest.log` and the day archives), as it writes them in its home.
pub fn logs_dir() -> PathBuf {
    game_home().join("logs")
}

/// Where the game writes `crash-<date>_<time>-client.txt` when it crashes.
pub fn crash_reports_dir() -> PathBuf {
    game_home().join("crash-reports")
}

/// The crash reports in `dir`, oldest first (their names sort by time).
fn crash_reports_in(dir: &Path) -> Vec<PathBuf> {
    let mut reports: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    reports.sort();
    reports
}

/// The newest crash report, if any.
pub fn newest_crash_report() -> Option<PathBuf> {
    crash_reports_in(&crash_reports_dir()).pop()
}

/// The newest crash report the game playing in `home` wrote at or after `since` (one from the
/// run that started then).
pub fn crash_report_since(home: &Path, since: std::time::SystemTime) -> Option<PathBuf> {
    crash_report_in_since(&home.join("crash-reports"), since)
}

fn crash_report_in_since(dir: &Path, since: std::time::SystemTime) -> Option<PathBuf> {
    crash_reports_in(dir).into_iter().rev().find(|p| {
        fs::metadata(p)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= since)
    })
}

/// The error a crash report describes, on one line: the lines between its description and the
/// stack (`panicked at src/game.rs:10:5: boom`).
pub fn crash_summary(report: &str) -> Option<String> {
    let mut lines = report
        .lines()
        .skip_while(|l| !l.starts_with("Description:"));
    lines.next()?;
    let error: Vec<&str> = lines
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty() && !l.starts_with('\t'))
        .map(str::trim)
        .collect();
    (!error.is_empty()).then(|| error.join(" "))
}

/// The line the game prints when it has drawn its first frame.
pub const GAME_FIRST_FRAME: &str = "Skyline: first frame";

/// Whether a game's output ends in a crash while its Vulkan renderer started: an adapter line
/// (`Skyline renderer: <GPU> (Vulkan, <type>)`) and no first frame. A native crash in a graphics
/// driver leaves no crash report, only those lines (#746).
pub fn vulkan_start_crashed<S: AsRef<str>>(lines: impl IntoIterator<Item = S>) -> bool {
    let mut vulkan = false;
    for line in lines {
        let line = line.as_ref();
        if line.starts_with(GAME_FIRST_FRAME) {
            return false;
        }
        if let Some(adapter) = line.strip_prefix("Skyline renderer: ") {
            // The adapter line, not one of the start's steps: "<GPU> (<backend>, <type>)".
            if adapter.ends_with(')') && adapter.contains(" (") {
                vulkan = adapter.contains(" (Vulkan, ");
            }
        }
    }
    vulkan
}

/// The game's graphics settings file in its home (the game's `graphics.rs`).
const GRAPHICS_FILE: &str = "skyline-graphics.txt";

/// Sets `graphicsApi:dx12` in `home`'s graphics settings when it is unset or `auto`, keeping
/// every other line. True when it changed; a player's own `vulkan` or `dx12` stays.
pub fn switch_to_dx12(home: &Path) -> Result<bool, String> {
    let path = home.join(GRAPHICS_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut lines: Vec<String> = Vec::new();
    let mut found = false;
    for line in text.lines() {
        match line.split_once(':') {
            Some((key, value)) if key.trim() == "graphicsApi" => {
                if value.trim() != "auto" {
                    return Ok(false);
                }
                found = true;
                lines.push("graphicsApi:dx12".to_owned());
            }
            _ => lines.push(line.to_owned()),
        }
    }
    if !found {
        lines.push("graphicsApi:dx12".to_owned());
    }
    fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    fs::write(&path, lines.join("\n") + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(true)
}

/// The last `lines` lines of `file`, or why it cannot be read.
fn tail_of(file: &Path, lines: usize) -> Result<String, String> {
    let bytes = fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..].join("\n") + "\n")
}

/// `logs`: the end of the game's latest.log, or with `crash` its newest crash report. The path
/// read comes first.
pub fn read_logs(lines: usize, crash: bool) -> Result<(PathBuf, String), String> {
    if crash {
        let path = newest_crash_report()
            .ok_or_else(|| format!("No crash report in {}.", crash_reports_dir().display()))?;
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok((path, text));
    }
    let path = logs_dir().join("latest.log");
    let text = tail_of(&path, lines.max(1))?;
    Ok((path, text))
}

/// Splits the player's extra game arguments (Settings' one line) into words, the way a shell
/// would: spaces separate them, single or double quotes group a word with spaces in it, and a
/// backslash escapes the next character (outside quotes, or inside double quotes).
pub fn split_args(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' {
                    // Inside double quotes only `"` and `\` are escaped, as a shell does.
                    match chars.next() {
                        Some(n @ ('"' | '\\')) => word.push(n),
                        Some(n) => {
                            word.push('\\');
                            word.push(n);
                        }
                        None => word.push('\\'),
                    }
                } else {
                    word.push(c);
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    started = true;
                }
                '\\' => {
                    if let Some(n) = chars.next() {
                        word.push(n);
                        started = true;
                    }
                }
                c if c.is_whitespace() => {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                _ => {
                    word.push(c);
                    started = true;
                }
            },
        }
    }
    if started {
        words.push(word);
    }
    words
}

/// The extra game arguments the player set in Settings, split into words.
pub fn game_args() -> Vec<String> {
    split_args(&Settings::load().game_args)
}

/// Appends the player's extra game arguments, so they add to the launcher's own or override them.
pub fn append_game_args(cmd: &mut Command) {
    cmd.args(game_args());
}

pub fn client_command(tag: &str) -> Command {
    let home = game_home();
    let _ = fs::create_dir_all(&home);
    let mut cmd = Command::new(client_program(tag));
    cmd.current_dir(home).arg("--resources").arg(resources(tag));
    // PipeWire remembers volume and mute per application name. The game's own name is
    // "PipeWire ALSA [horizon-client]", shared with development and test runs, where
    // muting it once left every later game silent: played from here it is "Skyline".
    if std::env::var_os("PIPEWIRE_PROPS").is_none() {
        cmd.env(
            "PIPEWIRE_PROPS",
            "{ application.name = \"Skyline\" media.role = \"Game\" }",
        );
    }
    cmd
}

/// The game's program: on macOS the one inside the version's Skyline.app, so the Dock and Cmd+Tab
/// show Skyline's name and icon (game_app.rs); the bare binary elsewhere, or if that fails.
fn client_program(tag: &str) -> PathBuf {
    let dir = version_dir(tag);
    #[cfg(target_os = "macos")]
    match crate::game_app::ensure(&dir) {
        Ok(program) => return program,
        Err(e) => eprintln!("Starting the game outside {}: {e}", crate::game_app::APP),
    }
    dir.join(CLIENT)
}

pub fn server_command(tag: &str, folder: &Path) -> Command {
    let mut cmd = Command::new(version_dir(tag).join(CLIENT));
    cmd.current_dir(folder)
        .arg("--resources")
        .arg(resources(tag))
        .args(["--dedicated", "--nogui"]);
    cmd
}

pub fn eula_agreed(folder: &Path) -> bool {
    fs::read_to_string(folder.join("eula.txt"))
        .is_ok_and(|s| s.lines().any(|l| l.trim() == "eula=true"))
}

/// Writes eula.txt as vanilla's server expects it, once the player agreed in the launcher.
pub fn agree_eula(folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let text = "#By changing the setting below to TRUE you are indicating your agreement to our EULA (https://aka.ms/MinecraftEULA).\neula=true\n";
    fs::write(folder.join("eula.txt"), text).map_err(|e| e.to_string())
}

// ----- Shared steps for the front ends -----------------------------------------------------------

/// The installed version to use: the one asked for, else the selected one, else the newest.
pub fn resolve_version(requested: Option<&str>) -> Result<String, String> {
    let installed = installed();
    if let Some(tag) = requested {
        return installed
            .iter()
            .find(|t| *t == tag)
            .cloned()
            .ok_or_else(|| format!("{tag} is not installed."));
    }
    let selected = Settings::load().selected.filter(|s| installed.contains(s));
    selected
        .or_else(|| installed.first().cloned())
        .ok_or_else(|| "No version is installed yet.".into())
}

/// Removes an installed version and moves the selection to the newest one left if needed.
pub fn remove_version(tag: &str) -> Result<(), String> {
    if !installed().iter().any(|t| t == tag) {
        return Err(format!("{tag} is not installed."));
    }
    remove(tag)?;
    let mut settings = Settings::load();
    if settings.selected.as_deref() == Some(tag) {
        settings.selected = installed().first().cloned();
        settings.save();
    }
    Ok(())
}

/// Whether a version has the game's program (a version from before the rename has
/// `horizon-client` instead, which this launcher doesn't start).
pub fn has_program(tag: &str) -> bool {
    version_dir(tag).join(CLIENT).exists()
}

/// The programs a Skyline build folder holds, the game's first.
const PROGRAMS: [&str; 4] = [
    "skyline",
    "skyline-ctl",
    "skyline-prepare",
    "skyline-server",
];

/// Where a picked folder keeps the built game: the folder itself, or its `target/release` (a
/// source checkout).
fn build_folder(picked: &Path) -> Option<PathBuf> {
    let exe = |name: &str| {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    };
    [picked.to_path_buf(), picked.join("target").join("release")]
        .into_iter()
        .find(|dir| dir.join(exe(PROGRAMS[0])).is_file())
}

/// A version name for a picked folder: its own name (a checkout's, not `release`), made safe for
/// a folder and not taken yet.
fn added_name(picked: &Path, taken: &[String]) -> String {
    let base: String = picked
        .components()
        .rev()
        .filter_map(|c| c.as_os_str().to_str())
        .find(|n| !matches!(*n, "release" | "debug" | "target" | "bin"))
        .unwrap_or("local")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let base = if base.is_empty() {
        "local".into()
    } else {
        base
    };
    (1..)
        .map(|n| {
            if n == 1 {
                base.clone()
            } else {
                format!("{base}-{n}")
            }
        })
        .find(|n| !taken.contains(n) && !version_dir(n).exists())
        .unwrap_or(base)
}

/// Adds a Skyline build from a folder on this computer as a version: its programs are linked
/// (copied where links can't be made), with the checkout's `research-lock.json` when it has one.
/// Returns the new version's name.
pub fn add_version_from_folder(picked: &Path) -> Result<String, String> {
    let build = build_folder(picked).ok_or_else(|| {
        format!(
            "No Skyline game in {}: pick the folder with the `skyline` program (or a source checkout built with cargo build --release).",
            picked.display()
        )
    })?;
    let name = added_name(picked, &installed());
    let dir = version_dir(&name);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let link = |from: &Path, to: &Path| -> Result<(), String> {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(from, to);
        #[cfg(not(unix))]
        let made = fs::copy(from, to).map(|_| ());
        made.map_err(|e| format!("{}: {e}", to.display()))
    };
    for program in PROGRAMS {
        let file = if cfg!(windows) {
            format!("{program}.exe")
        } else {
            program.to_string()
        };
        let from = build.join(&file);
        if from.is_file() {
            link(&from, &dir.join(&file))?;
        }
    }
    if let Some(lock) = build
        .ancestors()
        .map(|d| d.join("research-lock.json"))
        .find(|p| p.is_file())
    {
        link(&lock, &dir.join("research-lock.json"))?;
    }
    fs::write(dir.join(COMPLETE), "").map_err(|e| e.to_string())?;
    Ok(name)
}

/// The Minecraft folder to prepare from: the one given, the one chosen in the launcher, or the
/// official launcher's default. It may be absent when imported files can be reused.
pub fn minecraft_dir(requested: Option<PathBuf>) -> Result<PathBuf, String> {
    Ok(requested
        .or(Settings::load().minecraft_dir)
        .or_else(default_minecraft_dir)
        .unwrap_or_else(|| horizon_prepare::artifacts::minecraft_dir(None)))
}

/// The server folder: the one given, the one chosen in the launcher, or `server/` in the data folder.
pub fn server_dir(requested: Option<PathBuf>) -> PathBuf {
    requested
        .or(Settings::load().server_dir)
        .unwrap_or_else(|| data_dir().join("server"))
}

/// Whether `tag` is prepared, with the run-time reports it needs completed when another copy has them.
pub fn need_prepared(tag: &str) -> Result<(), String> {
    if is_prepared(tag) {
        complete_reports(&versions_dir(), tag);
        Ok(())
    } else {
        Err(format!(
            "{tag} is not prepared yet. Prepare it first (it needs your own Minecraft {}).",
            game_version(tag)
        ))
    }
}

/// The client's command, once its resources are prepared.
pub fn client(tag: &str) -> Result<Command, String> {
    need_prepared(tag)?;
    complete_assets(tag);
    let mut cmd = client_command(tag);
    append_game_args(&mut cmd);
    Ok(cmd)
}

/// The dedicated server's command in `folder`, once prepared and the EULA is agreed to.
pub fn server(tag: &str, folder: &Path, accept_eula: bool) -> Result<Command, String> {
    need_prepared(tag)?;
    if accept_eula {
        agree_eula(folder)?;
    }
    if !eula_agreed(folder) {
        return Err("Running a server needs your agreement to the Minecraft EULA (https://aka.ms/MinecraftEULA). Accept it to continue.".into());
    }
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    Ok(server_command(tag, folder))
}

/// A process whose output is kept: the last `LOG_KEEP` lines, and its result once it ends.
pub struct Logged {
    pub running: Running,
    pub log: Arc<Mutex<std::collections::VecDeque<String>>>,
    pub exit: Arc<Mutex<Option<Result<(), String>>>>,
}

pub const LOG_KEEP: usize = 2000;

impl Logged {
    pub fn start(job: Job, cmd: Command) -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let running = spawn(job, cmd, tx)?;
        let log = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let exit = Arc::new(Mutex::new(None));
        let (l, e) = (log.clone(), exit.clone());
        std::thread::spawn(move || {
            for msg in rx {
                match msg {
                    Msg::Line(_, line) => {
                        let mut log = l.lock().unwrap();
                        log.push_back(line);
                        if log.len() > LOG_KEEP {
                            log.pop_front();
                        }
                    }
                    Msg::Exit(_, result) => *e.lock().unwrap() = Some(result),
                    _ => {}
                }
            }
        });
        Ok(Self { running, log, exit })
    }

    pub fn finished(&self) -> Option<Result<(), String>> {
        self.exit.lock().unwrap().clone()
    }

    pub fn tail(&self, lines: usize) -> Vec<String> {
        let log = self.log.lock().unwrap();
        log.iter()
            .skip(log.len().saturating_sub(lines))
            .cloned()
            .collect()
    }

    pub fn wait(&self) -> Result<(), String> {
        loop {
            if let Some(result) = self.finished() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Asks a server to stop, and kills it if it has not after `grace`.
    pub fn stop(&mut self, grace: std::time::Duration) {
        self.running.send("stop");
        let start = std::time::Instant::now();
        while self.finished().is_none() && start.elapsed() < grace {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if self.finished().is_none() {
            self.running.kill();
        }
    }
}

/// The launcher's own path, read once at start.
pub fn exe_path() -> &'static Path {
    static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().unwrap_or_else(|_| PathBuf::from(LAUNCHER_BINARY)))
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Job {
    Prepare,
    Client,
    Server,
}

pub enum Msg {
    /// A version's files are gone (or could not be deleted).
    Removed(String, Result<(), String>),
    Line(Job, String),
    Exit(Job, Result<(), String>),
    /// `reuse_prepared_everywhere` prepared these versions.
    Reused(Vec<(String, String)>),
    /// The game about to start is prepared and its assets complete: it can be started.
    Ready,
}

/// A running process; its output arrives as `Msg::Line`, its end as `Msg::Exit`.
pub struct Running {
    pub child: Arc<Mutex<Child>>,
    pub stdin: Option<ChildStdin>,
}

impl Running {
    pub fn send(&mut self, line: &str) {
        if let Some(stdin) = &mut self.stdin {
            let _ = writeln!(stdin, "{line}");
            let _ = stdin.flush();
        }
    }
    pub fn kill(&self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

pub fn spawn(job: Job, mut cmd: Command, tx: Sender<Msg>) -> Result<Running, String> {
    crate::process::hide_console(&mut cmd);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start {:?}: {e}", cmd.get_program()))?;
    let stdin = child.stdin.take();
    let mut readers = Vec::new();
    let out = child
        .stdout
        .take()
        .map(|o| Box::new(o) as Box<dyn Read + Send>);
    let err = child
        .stderr
        .take()
        .map(|o| Box::new(o) as Box<dyn Read + Send>);
    for stream in [out, err].into_iter().flatten() {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                let _ = tx.send(Msg::Line(job, line));
            }
        }));
    }
    let child = Arc::new(Mutex::new(child));
    let waited = child.clone();
    std::thread::spawn(move || {
        for r in readers {
            let _ = r.join();
        }
        let status = loop {
            if let Some(s) = waited.lock().unwrap().try_wait().transpose() {
                break s;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let result = match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("Exited with {s}")),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(Msg::Exit(job, result));
    });
    Ok(Running { child, stdin })
}

/// The launcher's appearance, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Dark,
    Light,
    /// Follow the system's light or dark.
    System,
}

/// What the launcher's window does while the game runs, as the player picks it in Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WhilePlaying {
    /// The window stays where it is.
    #[default]
    Stay,
    /// The window goes to the Dock or the taskbar, and comes back when the game closes.
    Minimize,
    /// The launcher hides (macOS only: elsewhere it minimises), and comes back when the game
    /// closes.
    Hide,
}

impl WhilePlaying {
    pub const ALL: [WhilePlaying; 3] = [Self::Stay, Self::Minimize, Self::Hide];

    /// Its name in `settings.json` and on the command line.
    pub fn name(self) -> &'static str {
        match self {
            Self::Stay => "stay",
            Self::Minimize => "minimize",
            Self::Hide => "hide",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        let name = match name.as_str() {
            "minimise" => "minimize",
            "keep" | "open" | "off" => "stay",
            n => n,
        };
        Self::ALL.into_iter().find(|w| w.name() == name)
    }

    /// What this computer does for it: only macOS can hide a whole app.
    pub fn here(self) -> Self {
        match self {
            Self::Hide if !cfg!(target_os = "macos") => Self::Minimize,
            w => w,
        }
    }
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub selected: Option<String>,
    /// The instance Play starts (`instances`), by id. Unset means Skyline, the game's home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    pub minecraft_dir: Option<PathBuf>,
    pub server_dir: Option<PathBuf>,
    /// Off by default, so the opening animation shows unless the player turns it off.
    #[serde(default)]
    pub skip_opening: bool,
    /// Linux: whether the launcher keeps its entry in the applications menu (`desktop`). Unset
    /// means yes; removing the entry stores `false`, so it doesn't come back on the next start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<bool>,
    /// Extra arguments for the game's command line, as one line (see [`split_args`]). Empty by
    /// default. Appended after the launcher's own arguments, so they can add to or override them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game_args: String,
    /// Dark, Light, or the system's: the launcher's look. Dark by default.
    #[serde(default)]
    pub appearance: Appearance,
    /// What the window does while the game runs. Stays by default.
    #[serde(default)]
    pub while_playing: WhilePlaying,
    /// The accent colour on top of the appearance (`themes::Accent`). Skyline's red by default.
    /// Kept under `skyline_accent`: the old `accent` key's violet was the old default (`parse`).
    #[serde(default, rename = "skyline_accent")]
    pub accent: crate::themes::Accent,
    /// Home's background: a picture the player picked, kept in the data folder under this name
    /// (`themes::keep_background`). Unset shows Skyline's landscape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// The window's language. Unset follows the system's (`i18n::Language::system`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<crate::i18n::Language>,
}

impl Settings {
    pub fn load() -> Self {
        Self::parse(&fs::read_to_string(data_dir().join("settings.json")).unwrap_or_default())
    }
    pub fn parse(text: &str) -> Self {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Self::default();
        };
        if let Some(fields) = value.as_object_mut() {
            // An appearance from a newer launcher is Dark, not a reason to lose every setting.
            if fields
                .get("appearance")
                .is_some_and(|a| serde_json::from_value::<Appearance>(a.clone()).is_err())
            {
                fields.remove("appearance");
            }
            if fields
                .get("while_playing")
                .is_some_and(|w| serde_json::from_value::<WhilePlaying>(w.clone()).is_err())
            {
                fields.remove("while_playing");
            }
            // From before the rename: violet was the default and becomes red; a picked hue stays.
            if let Some(old) = fields.remove("accent") {
                if !fields.contains_key("skyline_accent") && old.as_str() != Some("violet") {
                    fields.insert("skyline_accent".into(), old);
                }
            }
            if fields.get("skyline_accent").is_some_and(|a| {
                serde_json::from_value::<crate::themes::Accent>(a.clone()).is_err()
            }) {
                fields.remove("skyline_accent");
            }
            // A language this launcher doesn't have yet follows the system's.
            if fields.get("language").is_some_and(|l| {
                serde_json::from_value::<Option<crate::i18n::Language>>(l.clone()).is_err()
            }) {
                fields.remove("language");
            }
        }
        serde_json::from_value(value).unwrap_or_default()
    }
    /// Whether the launcher keeps its entry in the applications menu (Linux).
    pub fn desktop_entry(&self) -> bool {
        self.desktop_entry.unwrap_or(true)
    }
    /// The window's language: the one picked, else the system's.
    pub fn language(&self) -> crate::i18n::Language {
        self.language.unwrap_or_else(crate::i18n::Language::system)
    }
    pub fn save(&self) {
        let _ = fs::create_dir_all(data_dir());
        let _ = fs::write(
            data_dir().join("settings.json"),
            serde_json::to_string_pretty(self).unwrap(),
        );
    }
}

#[cfg(test)]
#[path = "backend/java_tests.rs"]
mod java_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    /// The game program's mod API level is its mark's; a program from before the mark (0.2.68
    /// and older) or none at all serves level 0.
    #[test]
    fn a_game_program_says_its_mod_api_level() {
        let dir = std::env::temp_dir().join(format!("skyline-api-level-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let marked = dir.join("marked");
        let mut program = b"\x7fELF code code".to_vec();
        program.extend_from_slice(b"horizon-mod-api-level:0007;");
        fs::write(&marked, &program).unwrap();
        let old = dir.join("old");
        fs::write(&old, b"\x7fELF code without a mark").unwrap();
        assert_eq!(program_mod_api_level(&marked), 7);
        assert_eq!(program_mod_api_level(&old), 0);
        assert_eq!(program_mod_api_level(&dir.join("missing")), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// A preparation that reuses the imported game files still reads the assets from the
    /// launcher that has them: a Prism Launcher instance's are in the launcher root, linked into
    /// the official layout (jan.sonki on Discord: silent, grey title screen since 0.2.48).
    #[test]
    fn reused_imports_take_the_assets_from_the_launcher_that_has_them() {
        let dir =
            std::env::temp_dir().join(format!("skyline-reused-assets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let client = b"fake client jar";
        let manifest = br#"{"id": "26.3"}"#;
        let write = |path: &str, data: &[u8]| {
            let path = dir.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        let prism = "PrismLauncher";
        write(
            &format!("{prism}/libraries/com/mojang/minecraft/26.3/minecraft-26.3-client.jar"),
            client,
        );
        write(&format!("{prism}/assets/indexes/34.json"), b"{}");
        let instance = dir.join(prism).join("instances/26.3/minecraft");
        fs::create_dir_all(&instance).unwrap();
        let pins = sources::Pins::read(&serde_json::json!({ "minecraft": {
            "version": "26.3",
            "client_sha1": sources::sha1_hex(client),
            "manifest_sha1": sources::sha1_hex(manifest),
        }}))
        .unwrap();
        let linked = dir.join("linked");
        let fetch = |_: &str| Ok(manifest.to_vec());
        let lines = std::cell::RefCell::new(Vec::new());
        let log = |line: String| lines.borrow_mut().push(line);
        let resolved = sources::resolve(&instance, &pins, &[], &linked, &fetch, &log);
        let folder = assets_folder(resolved, &instance, "26.3", &log);
        assert_eq!(folder, linked);
        assert!(folder.join("assets/indexes/34.json").is_file());

        // Nothing found: the chosen folder, and a line saying what the game will lack.
        let empty = dir.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let resolved = sources::resolve(&empty, &pins, &[], &linked, &fetch, &log);
        assert_eq!(assets_folder(resolved, &empty, "26.3", &log), empty);
        assert!(lines.borrow().last().unwrap().contains("will be silent"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resources_without_sounds_are_reported() {
        let dir = std::env::temp_dir().join(format!("skyline-no-sounds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets/minecraft/textures")).unwrap();
        fs::write(dir.join(RESOURCES_MANIFEST), "{}").unwrap();
        assert!(!sounds_in(&dir));
        fs::write(dir.join("assets/minecraft/sounds.json"), "{}").unwrap();
        assert!(sounds_in(&dir));
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            resources_summary(true, true, "26.3"),
            "Prepared for Minecraft 26.3"
        );
        let silent = resources_summary(true, false, "26.3");
        assert!(silent.contains("without its sounds") && silent.contains("Prepare again"));
        // Not prepared says only that, sounds or not.
        assert_eq!(
            resources_summary(false, false, "26.3"),
            resources_summary(false, true, "26.3")
        );
    }

    #[test]
    fn the_game_plays_under_its_own_audio_stream_name() {
        if std::env::var_os("PIPEWIRE_PROPS").is_some() {
            return;
        }
        let cmd = client_command("v2026.10.13");
        let props = cmd
            .get_envs()
            .find(|(k, _)| *k == "PIPEWIRE_PROPS")
            .and_then(|(_, v)| v);
        assert!(props.is_some_and(|v| v
            .to_string_lossy()
            .contains("application.name = \"Skyline\"")));
    }

    #[test]
    fn a_crash_after_the_vulkan_adapter_line_and_before_the_first_frame_is_a_start_crash() {
        let start = [
            "Skyline: resources loaded in 11.3s; 35723 block states, 1658 items, 67 biomes",
            "Skyline renderer: Intel(R) HD Graphics 530 (Vulkan, IntegratedGpu)",
        ];
        assert!(vulkan_start_crashed(start));
        // A step line after the adapter keeps it a start crash.
        let step = [start[1], "Skyline renderer: creating the pipelines"];
        assert!(vulkan_start_crashed(step));
        // A crash after the first frame is the game's, not the renderer start's.
        let later = [start[1], "Skyline: first frame after 812 ms"];
        assert!(!vulkan_start_crashed(later));
        // Already on DirectX 12, or no renderer yet: nothing to switch.
        let dx12 = ["Skyline renderer: Intel(R) HD Graphics 530 (Dx12, IntegratedGpu)"];
        assert!(!vulkan_start_crashed(dx12));
        assert!(!vulkan_start_crashed(&start[..1]));
        // Vulkan failed to open and DirectX 12 took over: its line is the last adapter.
        assert!(!vulkan_start_crashed([start[1], dx12[0]]));
    }

    #[test]
    fn a_renderer_start_crash_switches_auto_to_dx12_once() {
        let home = std::env::temp_dir().join(format!("skyline-dx12-switch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        // No settings file yet: one with the API alone.
        assert_eq!(switch_to_dx12(&home), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "graphicsApi:dx12\n"
        );
        // Already DirectX 12, or the player's own Vulkan: left alone.
        assert_eq!(switch_to_dx12(&home), Ok(false));
        fs::write(home.join(GRAPHICS_FILE), "sky:false\ngraphicsApi:vulkan\n").unwrap();
        assert_eq!(switch_to_dx12(&home), Ok(false));
        // Auto becomes DirectX 12 and the other settings stay.
        fs::write(
            home.join(GRAPHICS_FILE),
            "sky:false\ngraphicsApi:auto\nfog:true\n",
        )
        .unwrap();
        assert_eq!(switch_to_dx12(&home), Ok(true));
        assert_eq!(
            fs::read_to_string(home.join(GRAPHICS_FILE)).unwrap(),
            "sky:false\ngraphicsApi:dx12\nfog:true\n"
        );
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn extra_game_arguments_split_like_a_shell() {
        assert!(split_args("").is_empty());
        assert!(split_args("   ").is_empty());
        assert_eq!(split_args("--server-thread"), ["--server-thread"]);
        assert_eq!(
            split_args("--render-distance 32 --server-thread"),
            ["--render-distance", "32", "--server-thread"]
        );
        // Quotes keep a word with spaces together; either kind works.
        assert_eq!(
            split_args("--name 'Two Words' --world \"My World\""),
            ["--name", "Two Words", "--world", "My World"]
        );
        // An empty quoted word is still a word.
        assert_eq!(split_args("--name \"\""), ["--name", ""]);
        // A backslash escapes a space outside quotes, and " and \ inside double quotes.
        assert_eq!(split_args("--name My\\ World"), ["--name", "My World"]);
        assert_eq!(split_args("a\\tb c"), ["atb", "c"]);
        assert_eq!(split_args("\"a\\\"b\" 'c\\d'"), ["a\"b", "c\\d"]);
    }

    #[test]
    fn the_game_arguments_setting_round_trips_and_reaches_the_command() {
        let settings = Settings::parse(r#"{"game_args":"--server-thread"}"#);
        assert_eq!(settings.game_args, "--server-thread");
        // Unset in an older settings.json: no arguments, and nothing is written for them.
        let old = Settings::parse(r#"{"installed":true}"#);
        assert_eq!(old.game_args, "");
        assert!(!serde_json::to_string(&old).unwrap().contains("game_args"));

        let mut cmd = Command::new("true");
        cmd.args(split_args("--server-thread --render-distance 40"));
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["--server-thread", "--render-distance", "40"]);
    }

    #[test]
    fn the_window_stays_while_playing_unless_picked() {
        assert_eq!(Settings::parse("{}").while_playing, WhilePlaying::Stay);
        let hide = Settings::parse(r#"{"while_playing": "hide", "game_args": "--x"}"#);
        assert_eq!(hide.while_playing, WhilePlaying::Hide);
        let future = Settings::parse(r#"{"while_playing": "fade", "game_args": "--x"}"#);
        assert_eq!(
            (future.while_playing, future.game_args.as_str()),
            (WhilePlaying::Stay, "--x")
        );
        assert_eq!(
            WhilePlaying::parse("Minimise"),
            Some(WhilePlaying::Minimize)
        );
        assert_eq!(WhilePlaying::parse("off"), Some(WhilePlaying::Stay));
        assert_eq!(WhilePlaying::parse("close"), None);
        for w in WhilePlaying::ALL {
            let text = serde_json::to_string(&w).unwrap();
            assert_eq!(text, format!("\"{}\"", w.name()));
        }
        let here = WhilePlaying::Hide.here();
        assert_eq!(here == WhilePlaying::Hide, cfg!(target_os = "macos"));
    }

    #[test]
    fn the_language_follows_the_system_unless_picked() {
        use crate::i18n::Language;
        assert_eq!(Settings::parse("{}").language, None);
        let fr = Settings::parse(r#"{"language": "fr"}"#);
        assert_eq!(fr.language(), Language::French);
        let future = Settings::parse(r#"{"language": "eo", "game_args": "--x"}"#);
        assert_eq!((future.language, future.game_args.as_str()), (None, "--x"));
        let saved = serde_json::to_value(Settings::default()).unwrap();
        assert!(saved.get("language").is_none(), "unset is left out");
    }

    #[test]
    fn the_accent_is_red_unless_picked_and_an_unknown_one_keeps_the_rest() {
        use crate::themes::Accent;
        assert_eq!(Settings::parse("{}").accent, Accent::Red);
        // The old key's violet was the old default: it becomes red. A picked hue stays.
        assert_eq!(
            Settings::parse(r#"{"accent": "violet"}"#).accent,
            Accent::Red
        );
        assert_eq!(
            Settings::parse(r#"{"skyline_accent": "violet"}"#).accent,
            Accent::Violet
        );
        let teal =
            Settings::parse(r#"{"accent": "teal", "background": "background-0123456789ab.png"}"#);
        assert_eq!(
            (teal.accent, teal.background.as_deref()),
            (Accent::Teal, Some("background-0123456789ab.png"))
        );
        let future = Settings::parse(r#"{"accent": "plaid", "game_args": "--x"}"#);
        assert_eq!(
            (future.accent, future.game_args.as_str()),
            (Accent::Red, "--x")
        );
    }

    #[test]
    fn the_appearance_is_dark_unless_picked_and_an_unknown_one_keeps_the_rest() {
        assert_eq!(Settings::parse("{}").appearance, Appearance::Dark);
        let light = Settings::parse(r#"{"appearance": "light", "game_args": "--x"}"#);
        assert_eq!(light.appearance, Appearance::Light);
        let future = Settings::parse(r#"{"appearance": "sepia", "game_args": "--x"}"#);
        assert_eq!(
            (future.appearance, future.game_args.as_str()),
            (Appearance::Dark, "--x")
        );
        let settings = Settings {
            appearance: Appearance::System,
            ..Settings::default()
        };
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains(r#""appearance":"system""#), "{text}");
    }

    #[test]
    fn the_old_update_channel_settings_are_dropped_and_the_rest_kept() {
        for old in [
            r#"{"update_channel": "alpha", "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"update_channel": "stable", "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"beta_builds": true, "game_args": "--x", "selected": "v0.2.0"}"#,
            r#"{"beta_builds": false, "update_channel": "nightly", "game_args": "--x", "selected": "v0.2.0"}"#,
        ] {
            let settings = Settings::parse(old);
            assert_eq!(settings.game_args, "--x", "{old}");
            assert_eq!(settings.selected.as_deref(), Some("v0.2.0"), "{old}");
            let saved = serde_json::to_value(&settings).unwrap();
            assert!(saved.get("update_channel").is_none(), "{old}");
            assert!(saved.get("beta_builds").is_none(), "{old}");
        }
    }

    #[test]
    fn the_version_is_the_one_the_release_workflow_injects() {
        assert_eq!(version_or(Some("0.4.2")), "0.4.2");
        assert_eq!(version_or(Some("")), env!("CARGO_PKG_VERSION"));
        assert_eq!(version_or(None), env!("CARGO_PKG_VERSION"));
        assert_eq!(
            VERSION,
            option_env!("SKYLINE_LAUNCHER_VERSION")
                .filter(|v| !v.is_empty())
                .unwrap_or(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare_versions("v2026.10.13", "v2026.10.12"), Greater);
        assert_eq!(compare_versions("v2026.9.30", "v2026.10.1"), Less);
        assert_eq!(compare_versions("v2026.10.13.1", "v2026.10.13"), Greater);
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("launcher-v1.2", "1.2.0"), Equal);
        assert_eq!(compare_versions("1.0.0-rc1", "1.0.0"), Equal);
    }

    #[test]
    fn a_new_version_reuses_resources_prepared_by_a_compatible_one() {
        let versions =
            std::env::temp_dir().join(format!("skyline-launcher-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&versions);
        let oracle = "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaHeightmapOracle.java";
        let camera = "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCameraOracle.java";
        let collision =
            "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java";
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let release = |tag: &str, minecraft: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": minecraft, "client_sha1": "c", "server_sha1": "s"}});
            write(tag, "research-lock.json", &lock.to_string());
            write(tag, "tools/prepare_native_client.py", "schema 7");
            write(tag, oracle, "heightmap");
            write(tag, collision, "collision");
            write(tag, camera, tag); // Not an exporter: it may change freely.
        };
        let prepared = |tag: &str, minecraft: &str| {
            let manifest =
                serde_json::json!({"schema": 7, "collision_exporters_sha256": {collision: "x"}});
            write(
                tag,
                &format!(".local/skyline/{minecraft}/{RESOURCES_MANIFEST}"),
                &manifest.to_string(),
            );
            write(
                tag,
                &format!(".local/skyline/{minecraft}/worldgen/biomes.json"),
                "{}",
            );
        };
        let is_prepared = |tag: &str| {
            resources_at(&versions.join(tag))
                .join(RESOURCES_MANIFEST)
                .exists()
        };

        release("v2026.10.13", "26.3");
        prepared("v2026.10.13", "26.3");
        release("v2026.10.12", "26.2");
        prepared("v2026.10.12", "26.2");
        release("v2026.10.13.1", "26.3");
        assert_eq!(
            reuse_prepared(&versions, "v2026.10.13.1"),
            Some("v2026.10.13".into())
        );
        assert!(is_prepared("v2026.10.13.1"));
        assert!(resources_at(&versions.join("v2026.10.13.1"))
            .join("worldgen/biomes.json")
            .exists());
        let left: Vec<_> = fs::read_dir(versions.join("v2026.10.13.1/.local/skyline"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, ["26.3"]);
        // Already prepared: nothing to do.
        assert_eq!(reuse_prepared(&versions, "v2026.10.13.1"), None);

        // Another Minecraft version, a changed exporter or a changed prepare script: prepare again.
        release("v2026.10.14", "26.4");
        assert_eq!(reuse_prepared(&versions, "v2026.10.14"), None);
        release("v2026.10.15", "26.3");
        write("v2026.10.15", collision, "collision, fixed");
        assert_eq!(reuse_prepared(&versions, "v2026.10.15"), None);
        release("v2026.10.16", "26.3");
        write("v2026.10.16", "tools/prepare_native_client.py", "schema 8");
        assert_eq!(reuse_prepared(&versions, "v2026.10.16"), None);
        assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));

        // A version an older launcher installed without preparing it is caught up by the sweep
        // that starting the window, `versions` and `update` run; the incompatible ones stay as
        // they are, and a second sweep has nothing to do.
        release("v2026.10.17", "26.3");
        assert_eq!(
            reuse_prepared_in(&versions),
            [("v2026.10.17".to_string(), "v2026.10.13.1".to_string())]
        );
        assert!(is_prepared("v2026.10.17"));
        assert!(!is_prepared("v2026.10.15") && !is_prepared("v2026.10.16"));
        assert!(reuse_prepared_in(&versions).is_empty());

        // Versions with skyline-prepare: its sources are the preparer, so the same sources
        // reuse each other's resources, and a change or a version without them prepares again.
        let crate_file = "crates/horizon-prepare/src/lib.rs";
        release("v2026.10.18", "26.3");
        write("v2026.10.18", crate_file, "rust preparer");
        assert_eq!(reuse_prepared(&versions, "v2026.10.18"), None);
        prepared("v2026.10.18", "26.3");
        release("v2026.10.19", "26.3");
        write("v2026.10.19", crate_file, "rust preparer");
        assert_eq!(
            reuse_prepared(&versions, "v2026.10.19"),
            Some("v2026.10.18".into())
        );
        release("v2026.10.20", "26.3");
        write("v2026.10.20", crate_file, "rust preparer, changed");
        assert_eq!(reuse_prepared(&versions, "v2026.10.20"), None);
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn prepared_resources_follow_the_preparers_output_version() {
        let versions = std::env::temp_dir().join(format!(
            "skyline-launcher-output-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&versions);
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let lib = "crates/horizon-prepare/src/lib.rs";
        // A preparer with its sources (as releases ship them) and the exporters it runs.
        let release = |tag: &str, code: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": "s"}});
            write(tag, "research-lock.json", &lock.to_string());
            write(tag, "crates/horizon-prepare/Cargo.toml", "[package]");
            write(tag, "crates/horizon-prepare/build.rs", "fn main() {}");
            write(tag, lib, code);
            for name in LEGACY_EXPORTERS {
                write(
                    tag,
                    &format!("mods/horizon/src/test/java/dev/horizonlab/parity/{name}"),
                    name,
                );
            }
        };
        let prepared = |tag: &str| {
            write(
                tag,
                &format!(".local/skyline/26.3/{RESOURCES_MANIFEST}"),
                r#"{"schema": 7}"#,
            );
            write(tag, ".local/skyline/26.3/facts.json", tag);
        };
        let facts = |tag: &str| {
            fs::read_to_string(resources_at(&versions.join(tag)).join("facts.json")).ok()
        };

        // The preparer's code changed, not what it makes: the resources are kept.
        release(
            "v0.3.0",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() {}",
        );
        prepared("v0.3.0");
        release(
            "v0.3.1",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() { faster() }",
        );
        assert_eq!(reuse_prepared(&versions, "v0.3.1"), Some("v0.3.0".into()));
        assert_eq!(facts("v0.3.1").as_deref(), Some("v0.3.0"));
        // Its output changed, so the number did: prepare again.
        release(
            "v0.3.2",
            "pub const OUTPUT_VERSION: u32 = 2;\nfn prepare() {}",
        );
        assert_eq!(reuse_prepared(&versions, "v0.3.2"), None);
        assert_eq!(facts("v0.3.2"), None);
        // Another Minecraft jar, same number: prepare again.
        release(
            "v0.3.3",
            "pub const OUTPUT_VERSION: u32 = 1;\nfn prepare() {}",
        );
        let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "other", "server_sha1": "s"}});
        write("v0.3.3", "research-lock.json", &lock.to_string());
        assert_eq!(reuse_prepared(&versions, "v0.3.3"), None);

        // A preparer from before the number is known by its fingerprint.
        release("v0.2.64", "fn prepare() {}");
        let fingerprint = legacy_fingerprint(&versions.join("v0.2.64")).unwrap();
        let legacy = [(fingerprint.as_str(), 1)];
        let at = |tag: &str| output_version_in(&versions.join(tag), &legacy);
        assert_eq!(at("v0.2.64"), Some(1));
        assert_eq!(at("v0.3.2"), Some(2));
        // Line endings as checked out don't count; an exporter's or the sources' change does.
        write(
            "v0.2.64",
            "mods/horizon/src/test/java/dev/horizonlab/parity/ParityData.java",
            "ParityData.java\r",
        );
        assert_eq!(at("v0.2.64"), Some(1));
        write(
            "v0.2.64",
            "mods/horizon/src/test/java/dev/horizonlab/parity/VanillaCollisionOracle.java",
            "changed",
        );
        assert_eq!(at("v0.2.64"), None);
        release("v0.2.65", "fn prepare() { changed() }");
        assert_eq!(at("v0.2.65"), None);
        // A folder without the preparer's sources has no number.
        assert_eq!(output_version_in(&versions.join("missing"), &legacy), None);
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn prepared_versions_get_the_run_time_reports_without_preparing_again() {
        let versions = std::env::temp_dir().join(format!(
            "skyline-launcher-reports-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&versions);
        let write = |tag: &str, path: &str, text: &str| {
            let path = versions.join(tag).join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        let release = |tag: &str, server: &str| {
            write(tag, COMPLETE, "");
            let lock = serde_json::json!({"minecraft": {"version": "26.3", "client_sha1": "c", "server_sha1": server}});
            write(tag, "research-lock.json", &lock.to_string());
            write(
                tag,
                &format!(".local/skyline/26.3/{RESOURCES_MANIFEST}"),
                "{}",
            );
        };
        let resources = |tag: &str| resources_at(&versions.join(tag));
        let datagen = |tag: &str, name: &str| {
            write(tag, &format!(".local/datagen/out/reports/{name}"), tag);
        };

        // Prepared before the reports shipped with the resources: its own scratch output.
        release("v2026.10.13", "s");
        datagen("v2026.10.13", "registries.json");
        datagen("v2026.10.13", "commands.json");
        // Resources copied from it: the reports come from the version that ran the generator.
        release("v2026.10.14", "s");
        // Another server jar: the reports of the others would be wrong, and it has none.
        release("v2026.10.15", "other");
        assert!(reuse_prepared_in(&versions).is_empty());
        for tag in ["v2026.10.13", "v2026.10.14"] {
            for name in RUNTIME_REPORTS {
                assert_eq!(
                    fs::read_to_string(resources(tag).join(name)).unwrap(),
                    "v2026.10.13"
                );
            }
        }
        assert!(!resources("v2026.10.15").join("registries.json").exists());
        assert!(!complete_reports(&versions, "v2026.10.15"));
        // Once there they stay as they are, even when the scratch output is gone.
        fs::remove_dir_all(versions.join("v2026.10.13/.local/datagen")).unwrap();
        assert!(complete_reports(&versions, "v2026.10.13"));
        let left: Vec<_> = fs::read_dir(resources("v2026.10.14"))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .filter(|n| n.contains("partial"))
            .collect();
        assert!(left.is_empty());
        let _ = fs::remove_dir_all(&versions);
    }

    #[test]
    fn prepared_versions_get_what_the_asset_store_adds_without_preparing_again() {
        let dir = std::env::temp_dir().join(format!(
            "skyline-launcher-packs-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let (versions, store, empty) = (dir.join("versions"), dir.join("store"), dir.join("none"));
        let write = |path: PathBuf, data: &[u8]| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, data).unwrap();
        };
        // An official launcher folder whose asset store holds the two packs, a panorama (fake
        // 2x2 PNG headers), Unifont, a language and the sound list.
        let background = "minecraft/textures/gui/title/background";
        let png = |n: u8| {
            let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x02\0\0\0\x02".to_vec();
            data.push(n);
            data
        };
        let mut assets: Vec<(String, Vec<u8>)> = horizon_prepare::BUILT_IN_PACKS
            .iter()
            .map(|name| (name.to_string(), name.as_bytes().to_vec()))
            .collect();
        for face in ["0", "1", "2", "3", "4", "5", "overlay"] {
            assets.push((
                format!("{background}/panorama_{face}.png"),
                png(face.as_bytes()[0]),
            ));
        }
        for name in [
            "minecraft/font/include/unifont.json",
            "minecraft/font/unifont.zip",
            "pack.mcmeta",
            "minecraft/lang/fr_fr.json",
            "minecraft/sounds.json",
        ] {
            assets.push((name.into(), format!("{{\"{name}\": 1}}").into_bytes()));
        }
        let mut index = serde_json::Map::new();
        for (name, data) in &assets {
            let hash = sources::sha1_hex(data);
            write(
                store.join("assets/objects").join(&hash[..2]).join(&hash),
                data,
            );
            index.insert(
                name.clone(),
                serde_json::json!({"hash": hash, "size": data.len()}),
            );
        }
        let index = serde_json::json!({ "objects": index }).to_string();
        write(store.join("assets/indexes/7.json"), index.as_bytes());
        let manifest = serde_json::json!({
            "id": "9.9",
            "assetIndex": {"id": "7", "sha1": sources::sha1_hex(index.as_bytes())},
            "downloads": {"client": {"sha1": "c"}, "server": {"sha1": "s"}},
            "javaVersion": {"majorVersion": 25},
        })
        .to_string();
        write(store.join("versions/9.9/9.9.json"), manifest.as_bytes());
        fs::create_dir_all(&empty).unwrap();
        // The client jar's 1x1 placeholder.
        let placeholder = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01";
        let release = |tag: &str, prepared: bool| {
            let root = versions.join(tag);
            write(root.join(COMPLETE), b"");
            let lock = serde_json::json!({"minecraft": {"version": "9.9", "java_major": 25,
                "client_sha1": "c", "server_sha1": "s", "manifest_sha1": sources::sha1_hex(manifest.as_bytes())}});
            write(root.join("research-lock.json"), lock.to_string().as_bytes());
            if prepared {
                let resources = resources_at(&root);
                write(resources.join(RESOURCES_MANIFEST), b"{}");
                write(resources.join("assets/minecraft/lang/en_us.json"), b"{}");
                write(
                    resources
                        .join("assets")
                        .join(background)
                        .join("panorama_0.png"),
                    placeholder,
                );
            }
        };
        let lines = std::cell::RefCell::new(Vec::new());
        let log = |line: String| lines.borrow_mut().push(line);
        let asset = |tag: &str, name: &str| {
            fs::read(resources_at(&versions.join(tag)).join("assets").join(name)).ok()
        };

        // Resources prepared without any of them: a folder without an asset store is skipped.
        release("v0.2.1", true);
        complete_assets_in(
            &versions,
            "v0.2.1",
            || vec![empty.clone(), store.clone()],
            &log,
        );
        assert_eq!(
            *lines.borrow(),
            [
                "Built-in pack assets ready: 2 files, 0.0 MB",
                "Panorama assets ready: 7 files, 0.0 MB",
                "Font assets ready: 2 files, 0.0 MB",
                "Language assets ready: 2 files, 0.0 MB",
                "Sound assets ready: 1 files, 0.0 MB",
            ]
        );
        for (name, data) in &assets {
            assert_eq!(asset("v0.2.1", name).as_ref(), Some(data), "{name}");
        }
        // Nothing else is prepared again.
        let resources = resources_at(&versions.join("v0.2.1"));
        assert_eq!(fs::read(resources.join(RESOURCES_MANIFEST)).unwrap(), b"{}");

        // No asset store anywhere: one line each, the version stays prepared, and no second try.
        lines.borrow_mut().clear();
        release("v0.2.2", true);
        complete_assets_in(&versions, "v0.2.2", || vec![empty.clone()], &log);
        complete_assets_in(&versions, "v0.2.2", || vec![empty.clone()], &log);
        let what = lines
            .borrow()
            .iter()
            .map(|line| line.split(" not added: ").next().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            what,
            [
                "v0.2.2: Programmer Art and High Contrast",
                "v0.2.2: The title panorama",
                "v0.2.2: The Unifont glyphs",
                "v0.2.2: The languages",
                "v0.2.2: The sounds",
            ]
        );
        assert!(resources_at(&versions.join("v0.2.2"))
            .join(RESOURCES_MANIFEST)
            .exists());
        assert_eq!(
            asset("v0.2.2", &format!("{background}/panorama_0.png")).as_deref(),
            Some(&placeholder[..])
        );

        // Not prepared, or complete: nothing to do, without even looking for a Minecraft folder.
        lines.borrow_mut().clear();
        release("v0.2.3", false);
        let unasked = || -> Vec<PathBuf> { unreachable!("no folder is needed") };
        complete_assets_in(&versions, "v0.2.3", unasked, &log);
        complete_assets_in(&versions, "v0.2.1", unasked, &log);
        assert!(lines.borrow().is_empty());
        assert!(!resources_at(&versions.join("v0.2.3")).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_reports_are_found_and_summarised() {
        let dir =
            std::env::temp_dir().join(format!("skyline-launcher-crash-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let start = std::time::SystemTime::now() - std::time::Duration::from_secs(2);
        assert!(crash_report_in_since(&dir, start).is_none());
        fs::create_dir_all(&dir).unwrap();
        let report = "---- Skyline Crash Report ----\n// Details below.\n\nTime: 2026-10-14 09:05:07\nDescription: Unexpected error\n\npanicked at src/game.rs:10:5:\nboom\n\t0: horizon_client::game::frame\n";
        fs::write(dir.join("crash-2026-10-14_09.05.07-client.txt"), report).unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        assert_eq!(
            crash_report_in_since(&dir, start).unwrap(),
            dir.join("crash-2026-10-14_09.05.07-client.txt")
        );
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
        assert!(crash_report_in_since(&dir, later).is_none());
        assert_eq!(
            crash_summary(report).as_deref(),
            Some("panicked at src/game.rs:10:5: boom")
        );
        assert_eq!(crash_summary("no report"), None);
        let log = dir.join("latest.log");
        fs::write(&log, "a\nb\nc\n").unwrap();
        assert_eq!(tail_of(&log, 2).unwrap(), "b\nc\n");
        assert_eq!(tail_of(&log, 100).unwrap(), "a\nb\nc\n");
        assert!(tail_of(&dir.join("missing.log"), 5).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
