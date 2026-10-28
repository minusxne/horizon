# Building Skyline

## Requirements

- Rust stable through rustup (`source ~/.cargo/env`), with clippy and rustfmt.
- Linux packages for the game and launcher: ALSA, udev, xkbcommon, Wayland,
  Vulkan headers and loader (on Debian: `libasound2-dev libudev-dev
  libxkbcommon-dev libwayland-dev libvulkan-dev`).
- A JDK (17 or newer) for `skyline-prepare`'s exporters. The Java runtime that
  Prism Launcher downloads works.
- `rsvg-convert` (`librsvg2-bin`) only to regenerate icons.

## Build

The game, prepare, ctl and server tools are in the root workspace; the
launcher is its own workspace in `launcher/`.

    cargo build --release -p horizon-client -p horizon-prepare -p horizon-server
    (cd launcher && cargo build --release)

Binaries land in `target/release/` (`skyline`, `skyline-ctl`,
`skyline-prepare`, `skyline-server`) and `launcher/target/release/`
(`skyline-launcher`).

Internal crate names keep their original `horizon-*` names; only what a
player sees is called Skyline.

## Prepare

    target/release/skyline-prepare --root . --prepare \
        --minecraft-dir ~/.var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher \
        --server-jar .local/server-jars/server-26.3.jar

This writes `.local/skyline/26.3`. Nothing in `.local/` is ever committed;
`python3 tools/check_public_tree.py` refuses Mojang files.

## Test

    cargo test -p horizon-client --lib --bins --tests
    (cd launcher && umask 022 && cargo test)
    (cd third_party/ureq && cargo test)

Run test games from a scratch folder with a scratch `SKYLINE_HOME` and
`XDG_*`, since the game writes into its working directory:

    target/release/skyline --offscreen --world-frames 300 --smoke-report report.json

## Design tokens and icons

    cargo run -p xtask -- tokens   # crates/horizon-theme -> tokens.css
    cargo run -p xtask -- icons    # docs/media/brand/*.svg -> packaging/icons

`icons` needs `rsvg-convert` (librsvg2-bin). Without it, `tools/bin/rsvg-convert` is a
small pycairo stand-in that handles the brand masters: `PATH=$PWD/tools/bin:$PATH cargo run -p xtask -- icons`.
