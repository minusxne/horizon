# Skyline

Skyline is Minecraft: Java Edition 26.3, rewritten in Rust, as an offline game.
It reads the same worlds, follows the same rules and generates the same terrain
as the Java game, but it never goes online: no accounts, no updates, no
telemetry, no outside links. You play it on the files from your own copy of
Minecraft 26.3.

It comes as two programs:

- `skyline`: the game.
- `skyline-launcher`: the home screen around it (instances, worlds, content,
  servers on your own network, settings).

Beside them sit `skyline-prepare` (builds the game's resources from your own
Minecraft), `skyline-server` (hosts a LAN server) and `skyline-ctl` (a local
scripting tool, only active when the game runs with `--control`).

## Building

See [docs/building.md](docs/building.md). In short:

    cargo build --release -p horizon-client -p horizon-prepare
    (cd launcher && cargo build --release)

## Preparing the resources

Skyline does not ship any Mojang files. It builds them once from an
installed Minecraft 26.3 (the official launcher's folder or a Prism Launcher
instance works) and a 26.3 server jar you already have:

    skyline-prepare --root . --prepare --minecraft-dir PATH --server-jar PATH

With Prism Launcher, `--minecraft-dir` is the instance's `minecraft` folder,
for example
`~/.local/share/PrismLauncher/instances/26.3/minecraft` (Flatpak:
`~/.var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/instances/26.3/minecraft`).
The server jar is the `server.jar` Prism or the official launcher downloads.

The launcher does the same from its Versions page: Add from folder (pick the
`target/release` folder you built), then Prepare, and it asks for your
Minecraft folder if it can't find one.

## Playing

Start `skyline-launcher`:

- Play: the selected instance, its recent worlds and the Play button.
- Content: the instance's Skyline mods (`.hmod` files and mod folders),
  resource and shader packs, and each world's data packs. Drop files on a
  section or use its Add button; nothing is downloaded.
- Servers: LAN servers you host on this computer, reachable on your own
  network.
- Versions: the Skyline builds on this computer.
- Settings: your offline player name, appearance, language, game arguments,
  what happens while you play, and where your data is.

You can also run `skyline` directly from the build folder.

## Where data lives

- Game data (worlds, options, packs, mods): `~/.local/share/skyline`
- Launcher data (instances, settings): `~/.local/share/skyline-launcher`
- Prepared resources: `.local/skyline/26.3` in the build folder

Older folders and files from before the rename are moved to the Skyline
names the first time Skyline starts.

## License

MIT, see [LICENSE](LICENSE).
