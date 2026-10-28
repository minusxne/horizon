//! The launcher's core, shared by the GUI, the CLI and the MCP server.
pub mod account;
pub mod backend;
pub mod datapacks;
pub mod desktop;
pub mod desktop_seams;
pub mod export;
#[cfg(unix)]
pub mod game_app;
pub mod game_log;
pub mod heads;
pub mod i18n;
pub mod import;
pub mod instances;
pub mod launch;
pub mod mods;
pub mod packs;
pub mod process;
pub mod screenshots;
pub mod server_list;
pub mod servers;
pub mod shortcut;
pub mod sources;
pub mod themes;
pub mod worlds;
