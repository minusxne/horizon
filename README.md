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

## Where data lives

- Game data (worlds, options, packs, mods): `~/.local/share/skyline`
- Launcher data (instances, settings): `~/.local/share/skyline-launcher`
- Prepared resources: `.local/skyline/26.3` in the build folder

Older folders and files from before the rename are moved to the Skyline
names the first time Skyline starts.

## License

MIT, see [LICENSE](LICENSE).
