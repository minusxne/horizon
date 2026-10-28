# Offline snapshot

This fork freezes Horizon at upstream `horizon-rust/horizon` commit `4db071e28` (2026-10-28) for
long-term local play. It stays on the version you build here and talks to nothing outside your
computer except Microsoft sign-in, if you choose to sign in.

## What changed

- **One network gate.** Every HTTP request in the game, the launcher and `horizon-server` goes
  through `ureq`, patched in `third_party/ureq` (`src/unit.rs`, `host_allowed`). It refuses every
  host but Microsoft sign-in, the Minecraft services behind it (profiles, skins, joining
  online-mode servers) and this computer. Redirects are checked hop by hop.
- **No updates.** The launcher never lists releases, installs new versions, installs nightlies
  or replaces itself (`launcher/src/backend.rs`: `release_list`, `self_update_allowed`,
  `auto_install`; `launcher/src/nightly.rs`).
- **No telemetry.** `horizon_telemetry::endpoint()` is always `None`: nothing is uploaded, no
  "playing" ping, and the launcher never asks for data collection.
- **No ads, no outside links.** The Sculk partner card and banner are gone. The launcher's links
  open only Microsoft's sign-in page, local files and a server's own local web panel
  (`launcher/src/main.rs`, `link_allowed`). The game opens no web link from chat or server
  links (`crates/horizon-client/src/game/packs.rs`, `open_uri`); copy the link instead.
- **Play without sign-in.** A launcher built without Horizon's Azure app
  (`HORIZON_MSA_CLIENT_ID`) plays offline from your own prepared Minecraft files, as
  `cargo run -p horizon-client` does. A build with it keeps the ownership check.

Still working: single player, LAN, joining servers by address, hosting servers here. Not working:
the mod browser, patch notes, the Versions page's downloads, "download Minecraft from Mojang",
Play with Friends' relay, sharing logs, Tailscale's web pages, server resource packs.

## Build

As in [README.md](README.md#build-from-source): `cargo build --release -p horizon-client`, and
`cargo build --release` in `launcher/`. Prepare resources once from your own Minecraft 26.3
install with `horizon-prepare`.
