# Skyline's mark and icons

**The mark.** A pixel city skyline in Skyline's red standing on a pale horizon line,
its reflection below at a third. It is drawn on whole units of a 16-unit grid, so it is
sharp at 16 px and holds at 1024 px. `tools/../launcher/src/main.rs` `mark()` draws the same
rects in the launcher.

## Masters (edit these)

| File | What |
|------|------|
| `icon.svg` | the app icon on the 16 grid: the full square with 22 % corners (Windows, Linux, favicons) |
| `icon-macos.svg` | the same tile on Apple's 1024 grid (824 px tile, 185 px corners, transparent margin) |
| `avatar.svg` | full-bleed square for services that round or crop the image (GitHub, Discord) |
| `mark.svg`, `mark-dark.svg` | the mark alone, for light and for dark backgrounds |
| 
## Generated (don't edit)

`cargo run -p xtask -- icons` (needs `rsvg-convert` from librsvg) renders the masters into:

| Output | Used by |
|--------|---------|
| `packaging/icons/hicolor/<size>/apps/skyline.png` (16–512) and `scalable/apps/skyline.svg` | Linux: copy the tree into `~/.local/share/icons/hicolor` (or `/usr/share/icons/hicolor`) |
| `packaging/linux/horizon-launcher.desktop`, `skyline.desktop` (hand-written) | Linux: the launcher installs both and the hicolor icons itself, under `$XDG_DATA_HOME` (`~/.local/share`), with its own path in `Exec` (`launcher/src/desktop.rs`; `skyline-launcher desktop-entry --remove` takes them out). Wayland matches the windows' app ids `skyline-launcher` and `horizon` |
| `packaging/icons/skyline.ico` | Windows: embedded in `horizon-client.exe`, its tools and `skyline-launcher.exe` as icon resource 1 (`build.rs` of each, Windows builds only) |
| `packaging/icons/skyline.icns` | macOS: `Skyline.app/Contents/Resources/skyline.icns` |
| `crates/horizon-theme/icons/skyline-64.png` | the game window's icon at run time (X11 and Windows title bars, `window::icon`) |
| `web/favicon.svg`, `web/favicon-32.png`, `web/apple-touch-icon.png` | the website (its `npm run assets` copies them into its `public/`) |
| `icon-1024.png`, `icon-macos-1024.png` | previews, store listings |
| `social/github-avatar-500.png` | GitHub: the organisation or account avatar |

The page in `social/` is a screenshot, at their exact size, with Playwright, from the repository's root:

```sh
d=$(mktemp -d) && npm install --prefix "$d" playwright && npx --prefix "$d" playwright install chromium
NODE_PATH="$d/node_modules" node -e 'const {chromium}=require("playwright");(async()=>{const b=await chromium.launch();for(const[f,w,h,o]of[["card",1280,640,"github-social-1280x640"]]){const p=await b.newPage({viewport:{width:w,height:h}});await p.goto("file://"+require("path").resolve("docs/media/brand/social/"+f+".html"));await p.waitForTimeout(300);await p.screenshot({path:"docs/media/brand/social/"+o+".png"})}await b.close()})()'
```

## Drawn in code

| Where | What |
|-------|------|
| `crates/horizon-client/src/title_edition.rs` | the R, U and S of "RUST EDITION" under the classic title screen's logo: Skyline's own glyphs (MIT), rectangles in that file. The banner is rebuilt at load time from the player's own `textures/gui/title/edition.png` (its "EDITION" and "T" carried over, its shades sampled), so no Mojang pixels are in the repository; a resource pack's banner is left as drawn |

## Uploading (the accounts are Ludovic's)

- **GitHub avatar**: github.com/settings/profile (or the organisation's settings), *Edit* the
  picture, upload `social/github-avatar-500.png`.
- **GitHub social preview**: the repository's *Settings › General › Social preview › Edit*,
  upload `social/github-social-1280x640.png`.
- **Discord** (the server icon and banner, Kira's avatar): they live in the private kira repo since 2026-10-15
  (`assets/`, its README's "Discord brand assets"): this repo holds only the game, the launcher and the website.
