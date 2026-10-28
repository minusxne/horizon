//! Skyline launcher: installs Skyline builds, prepares them from the player's own Minecraft,
//! and starts the client or a dedicated server.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod art;
mod blocks;
mod cli;
mod console;
mod console_page;
mod datapacks_page;
mod export_page;
mod import_page;
mod instances_page;
mod language_page;
mod launch_page;
mod line_edit;
mod multiplayer_page;
mod packs_page;
mod perf;
mod screenshots_page;
mod scrollbar;
mod servers_page;
mod shortcut_page;
mod switch_page;
mod theme;
mod themes_page;
mod titlebar;
mod while_playing;
mod worlds_page;

use gpui::{
    actions, div, ease_out_quint, prelude::*, px, relative, size, Animation, AnimationExt,
    AnyElement, App, Application, Bounds, ClickEvent, Context, Div, ElementId, FocusHandle,
    FontWeight, Hsla, KeyBinding, KeyDownEvent, Menu, MenuItem, PathPromptOptions, SharedString,
    Stateful, Subscription, Window, WindowBounds, WindowOptions,
};
use horizon_launcher::account::{self, Accounts};
use horizon_launcher::backend::{self, Job, Msg, Running, Settings};
use horizon_launcher::i18n::{self, tr, trf};
use horizon_launcher::instances;
use scrollbar::Scrollbar;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use theme::Theme;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Play,
    Versions,
    Servers,
    Settings,
}

impl Page {
    /// `SKYLINE_LAUNCHER_PAGE=servers` (or `versions`, `settings`) opens on that page, for
    /// screenshots; Home otherwise.
    fn from_env() -> Page {
        match std::env::var("SKYLINE_LAUNCHER_PAGE").as_deref() {
            Ok("servers") => Page::Servers,
            Ok("versions") => Page::Versions,
            Ok("settings") => Page::Settings,
            _ => Page::Play,
        }
    }

    /// Every page, with its name and icon: the top bar shows all but Settings, which has its
    /// own button.
    const ALL: [(Page, &'static str, &'static str); 4] = [
        (Page::Play, "Home", "home"),
        (Page::Servers, "Servers", "server"),
        (Page::Versions, "Versions", "archive"),
        (Page::Settings, "Settings", "sliders"),
    ];
}

/// The last game the launcher started ended badly: what it left behind.
#[derive(Clone)]
struct Crash {
    /// The crash report the run wrote, if it wrote one.
    report: Option<std::path::PathBuf>,
    /// One line on what went wrong: the report's error, else how the process ended.
    summary: String,
    copied: bool,
}

/// The opening: the wordmark holds at least `OPENING_HOLD` (until the release
/// list is in, at most `OPENING_MAX`), then the launcher appears over `OPENING_REVEAL`.
const OPENING_HOLD: Duration = horizon_theme::OPENING_HOLD;
const OPENING_MAX: Duration = horizon_theme::OPENING_MAX;
const OPENING_REVEAL: Duration = horizon_theme::OPENING_REVEAL;

/// The phase of the opening at one moment: how far the reveal has gone, 0 to 1.
struct Opening {
    /// Set on the first frame, so a slow start does not eat into the hold.
    started: Option<Instant>,
    reveal: Option<Instant>,
    focus: FocusHandle,
}

fn ease_out_cubic(x: f32) -> f32 {
    horizon_theme::ease_out_cubic(x)
}

const LOG_LINES: usize = 2000;

struct Launcher {
    theme: Theme,
    page: Page,
    /// How many times the page changed, for the page transition.
    switches: usize,
    content_scroll: Scrollbar,
    opening: Option<Opening>,
    installed: Vec<String>,
    /// The version being deleted on a background thread. It is already out of `installed`, so
    /// nothing can launch, select or prepare it, and its row can't install or remove it again.
    removing: Option<String>,
    install_error: Option<(String, String)>,
    confirm_remove: Option<String>,
    picker_open: bool,
    /// The top bar's quick switch of who plays (`switch_page.rs`).
    account_menu: bool,
    /// Why the last picture for Home's background was refused.
    background_error: Option<String>,
    settings: Settings,
    preparing: bool,
    prepare_log: VecDeque<String>,
    prepare_error: Option<String>,
    client: Option<Running>,
    client_error: Option<String>,
    /// When the running client started, to tell its crash report from older ones.
    client_started: Option<std::time::SystemTime>,
    /// The folder the running game keeps its settings in: the game home, or a modpack's.
    client_home: Option<std::path::PathBuf>,
    crash: Option<Crash>,
    /// The Servers page (`servers_page.rs`).
    servers: servers_page::Servers,
    /// The window's own focus, so Tab reaches the controls when nothing else has the keyboard.
    root_focus: FocusHandle,
    /// The offline names (accounts.json) and the one that plays.
    accounts: Accounts,
    /// The game is waiting for its missing files (`complete_assets`) before it starts.
    starting_client: Option<String>,
    /// Where the next start goes at once: `--world <folder>` or `--server <address>`.
    quick_play: Option<(&'static str, String)>,
    /// The instance the running game plays and when it started: its play time grows on exit.
    client_run: Option<(String, Instant)>,
    /// Settings' Game arguments field: the line, whether it has focus, and its focus handle.
    args_input: line_edit::LineEdit,
    args_focused: bool,
    args_focus: FocusHandle,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    _appearance: Subscription,
    /// The session's measurements (`perf.rs`), and when the steps they time began.
    perf: perf::Perf,
    prepare_started: Option<Instant>,
    play_pressed: Option<Instant>,
    /// `skyline-launcher benchmark` started this window to measure it.
    bench: Option<perf::Bench>,
    /// The instances, for the sidebar and Play, and their dialog (`instances_page.rs`).
    instances: instances_page::Instances,
    /// Home's Worlds section, of the selected instance (`worlds_page.rs`).
    worlds: worlds_page::Worlds,
    multiplayer: multiplayer_page::Multiplayer,
    screenshots: screenshots_page::Screenshots,
    game_packs: packs_page::Packs,
    world_packs: datapacks_page::WorldPacks,
    export: Option<export_page::ExportPanel>,
    launch: Option<launch_page::LaunchPanel>,
    importing: import_page::Importing,
    console: console_page::Console,
    /// What the window did when the game started (`while_playing.rs`).
    aside: while_playing::Aside,
    /// The selected instance's desktop shortcut (`shortcut_page.rs`).
    shortcuts: shortcut_page::Shortcuts,
    /// The name the game made up for an offline player (`skyline-player.txt`), for the top bar.
    offline_name: Option<String>,
    /// Why the last offline name couldn't be added or picked.
    name_error: Option<String>,
    /// The player's servers, for the sidebar until the Servers page has read them.
    side_servers: Vec<horizon_launcher::servers::Entry>,
    /// The pixel art made for this window (`art.rs`), and pictures downloaded by URL.
    art: art::Art,
    /// The window's scale, as of its last frame, for art drawn outside `render`'s reach.
    scale: f32,
}

impl Launcher {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = channel();
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.theme = this.settings_theme(window);
            cx.notify();
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            if this.update(cx, |this, cx| this.drain(cx)).is_err() {
                break;
            }
        })
        .detach();
        let mut settings = Settings::load();
        i18n::set(settings.language());
        if settings.minecraft_dir.is_none() {
            settings.minecraft_dir = backend::default_minecraft_dir();
        }
        // Release builds on Linux put their entry back in the applications menu when there is
        // none or it starts a program that is gone (a moved or deleted binary), never over one
        // that works; `SKYLINE_LAUNCHER_DESKTOP_ENTRY=1` does it in a development build.
        if cfg!(target_os = "linux")
            && settings.desktop_entry()
            && (!cfg!(debug_assertions)
                || std::env::var_os("SKYLINE_LAUNCHER_DESKTOP_ENTRY").is_some())
        {
            std::thread::spawn(|| {
                let _ = horizon_launcher::desktop::repair(backend::exe_path());
            });
        }
        let installed = backend::installed();
        if settings
            .selected
            .as_ref()
            .is_none_or(|s| !installed.contains(s))
        {
            settings.selected = installed.first().cloned();
        }
        // Pin the window's chrome (macOS title bar) to the choice before the theme reads
        // `window.appearance()`, so both draw the same look.
        titlebar::apply_appearance(window, settings.appearance);
        let mut this = Self {
            theme: Theme::for_appearance(settings.appearance, settings.accent, window.appearance()),
            page: Page::from_env(),
            switches: 0,
            content_scroll: Scrollbar::default(),
            opening: (!settings.skip_opening).then(|| {
                let focus = cx.focus_handle();
                window.focus(&focus);
                Opening {
                    started: None,
                    reveal: None,
                    focus,
                }
            }),
            installed,
            removing: None,
            install_error: None,
            confirm_remove: None,
            picker_open: false,
            account_menu: false,
            background_error: None,
            args_input: line_edit::LineEdit::new(settings.game_args.clone(), 1024, 38),
            args_focused: false,
            args_focus: cx.focus_handle(),
            settings,
            preparing: false,
            prepare_log: VecDeque::new(),
            prepare_error: None,
            client: None,
            client_error: None,
            client_started: None,
            client_home: None,
            crash: None,
            servers: servers_page::Servers::new(),
            root_focus: cx.focus_handle(),
            accounts: Accounts::load(),
            starting_client: None,
            quick_play: None,
            client_run: None,
            tx,
            rx,
            _appearance: appearance,
            perf: perf::Perf::new("session"),
            prepare_started: None,
            play_pressed: None,
            bench: perf::Bench::from_env(),
            instances: instances_page::Instances::default(),
            worlds: worlds_page::Worlds::default(),
            multiplayer: multiplayer_page::Multiplayer::default(),
            screenshots: screenshots_page::Screenshots::default(),
            game_packs: packs_page::Packs::default(),
            world_packs: datapacks_page::WorldPacks::default(),
            export: None,
            launch: None,
            importing: Default::default(),
            console: Default::default(),
            aside: while_playing::Aside::new(window.window_handle()),
            shortcuts: Default::default(),
            offline_name: offline_name(),
            name_error: None,
            side_servers: horizon_launcher::servers::list(),
            art: art::Art::default(),
            scale: window.scale_factor(),
        };
        this.reload_instances();
        if this.bench.is_some() {
            this.perf.keep_local_log(false);
        }
        let tx = this.tx.clone();
        std::thread::spawn(move || {
            let reused = backend::reuse_prepared_everywhere();
            if !reused.is_empty() {
                let _ = tx.send(Msg::Reused(reused));
            }
        });
        this
    }

    fn go(&mut self, page: Page) {
        // The game may have made up a name since: read it again (a small file, once a switch).
        self.offline_name = offline_name();
        if page != self.page {
            self.page = page;
            self.switches += 1;
            self.content_scroll = Scrollbar::default();
        }
        self.picker_open = false;
        self.account_menu = false;
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        self.perf.tick();
        match self.bench.as_mut().map(|b| b.tick(&self.perf)) {
            Some(perf::Step::Go(page)) => {
                self.go(Page::ALL[page % Page::ALL.len()].0);
                changed = true;
            }
            Some(perf::Step::Scroll) => {
                self.go(Page::Versions);
                changed = true;
            }
            Some(perf::Step::Quit) => cx.quit(),
            Some(perf::Step::Wait) | None => {}
        }
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => self.installed = backend::installed(),
                Msg::Ready => {
                    if let Some(tag) = self.starting_client.take() {
                        let mut cmd = backend::client_command(&tag);
                        self.accounts = Accounts::load();
                        let instance = instances::selected();
                        // The offline name that plays: kept in the instance's own
                        // skyline-player.txt too, so the game plays it however it starts.
                        if let Some(name) = self.accounts.name_arg() {
                            if let Err(e) = account::write_game_name(&instance.home, name) {
                                eprintln!("Player name not kept: {e}");
                            }
                            cmd.arg("--name").arg(name);
                        }
                        let profile = instances::apply(&mut cmd, &instance);
                        let run = Some(instance.id);
                        if let Err(e) = profile {
                            self.client_error = Some(e);
                            self.quick_play = None;
                            continue;
                        }
                        if let Some((flag, value)) = self.quick_play.take() {
                            cmd.arg(flag).arg(value);
                        }
                        backend::append_game_args(&mut cmd);
                        // Last, around every argument: the instance's pre-launch command,
                        // wrapper and variables.
                        let launch = run
                            .as_ref()
                            .and_then(|id| instances::list().into_iter().find(|i| &i.id == id))
                            .map(|i| i.launch)
                            .unwrap_or_default();
                        let cmd = horizon_launcher::launch::wrap(cmd, &launch);
                        self.client_home = cmd.get_current_dir().map(|d| d.to_path_buf());
                        match backend::spawn(Job::Client, cmd, self.tx.clone()) {
                            Ok(r) => {
                                self.client = Some(r);
                                self.client_run = run.map(|id| (id, Instant::now()));
                                self.step_aside(cx);
                            }
                            Err(e) => self.client_error = Some(e),
                        }
                    }
                }
                Msg::Removed(tag, result) => {
                    if self.removing.as_ref() == Some(&tag) {
                        self.removing = None;
                    }
                    self.installed = backend::installed();
                    self.settings.selected =
                        selection_after_removal(&self.installed, Settings::load().selected);
                    if let Err(e) = result {
                        self.install_error = Some((tag, e));
                    }
                }
                Msg::Line(job, line) => {
                    if job == Job::Client && line.starts_with(perf::GAME_FIRST_FRAME) {
                        if let Some(pressed) = self.play_pressed.take() {
                            self.perf
                                .record_since("launcher.play_to_game_frame_ms", pressed);
                        }
                    }
                    // The MCP server keeps its own server's log; the launcher's are the preparer's
                    // and the game's.
                    if job != Job::Server {
                        self.prepare_log.push_back(line);
                        if self.prepare_log.len() > LOG_LINES {
                            self.prepare_log.pop_front();
                        }
                    }
                }
                Msg::Exit(job, result) => {
                    let error = result.err();
                    match job {
                        Job::Prepare => {
                            self.preparing = false;
                            if let (None, Some(started)) = (&error, self.prepare_started.take()) {
                                self.perf.record_since("launcher.prepare_ms", started);
                            }
                            // A preparer process's exit status says little, so its last line
                            // follows; the launcher's own errors are full sentences already.
                            self.prepare_error = error.map(|e| {
                                let tail = self
                                    .prepare_log
                                    .iter()
                                    .rev()
                                    .find(|l| !l.trim().is_empty())
                                    .filter(|_| e.starts_with("Exited with"));
                                tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                            });
                        }
                        Job::Client => {
                            self.client = None;
                            self.come_back(cx);
                            // The game saved: last played, sizes and pictures changed.
                            self.worlds.stale();
                            self.multiplayer.stale();
                            self.screenshots.stale();
                            self.game_packs.stale();
                            if let Some((id, started)) = self.client_run.take() {
                                if let Err(e) = instances::record_play(&id, started.elapsed()) {
                                    eprintln!("Play time not saved: {e}");
                                }
                                self.reload_instances();
                            }
                            // The run's own game folder: an instance's or a pack's keeps its reports.
                            let home = self.client_home.clone().unwrap_or_else(backend::game_home);
                            let report = self
                                .client_started
                                .take()
                                .and_then(|since| backend::crash_report_since(&home, since));
                            if error.is_some() && report.is_none() && self.switched_to_dx12() {
                                self.client_error = Some(
                                    "Skyline's Vulkan renderer crashed while starting on this \
                                     graphics card, so the game now uses DirectX 12. Press Play \
                                     to start it again. Video Settings > Graphics API switches \
                                     back."
                                        .into(),
                                );
                            } else if error.is_some() || report.is_some() {
                                let summary = report
                                    .as_ref()
                                    .and_then(|p| std::fs::read_to_string(p).ok())
                                    .and_then(|text| backend::crash_summary(&text))
                                    .or_else(|| {
                                        let tail = self
                                            .prepare_log
                                            .iter()
                                            .rev()
                                            .find(|l| !l.trim().is_empty());
                                        error.as_ref().map(|e| {
                                            tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                                        })
                                    })
                                    .unwrap_or_default();
                                self.crash = Some(Crash {
                                    report,
                                    summary,
                                    copied: false,
                                });
                            }
                        }
                        Job::Server => {}
                    }
                }
            }
        }
        changed |= self.servers_tick();
        if changed {
            cx.notify();
        }
    }

    /// After a crash with no report: when the game's output ends while its Vulkan renderer
    /// started, a native crash in the graphics driver (#746), switches the game to DirectX 12
    /// (Windows only; only where the player left the graphics API on Auto). True when it did.
    fn switched_to_dx12(&self) -> bool {
        if !cfg!(windows) || !backend::vulkan_start_crashed(&self.prepare_log) {
            return false;
        }
        // The GPU is the same for every profile: the game home's setting and the crashed run's.
        let home = backend::game_home();
        let mut homes = vec![home.clone()];
        homes.extend(self.client_home.clone().filter(|h| *h != home));
        let mut switched = false;
        for home in homes {
            match backend::switch_to_dx12(&home) {
                Ok(changed) => switched |= changed,
                Err(e) => eprintln!("Skyline launcher: could not switch to DirectX 12: {e}"),
            }
        }
        switched
    }

    /// The version Play starts: the selected instance's own, else the launcher's.
    fn selected(&self) -> Option<&String> {
        self.selected_instance()
            .and_then(|i| self.version_of(i))
            .or_else(|| self.launcher_version())
    }

    /// The launcher's version: what instances without one of their own play.
    fn launcher_version(&self) -> Option<&String> {
        self.settings
            .selected
            .as_ref()
            .filter(|s| self.installed.contains(s))
    }

    /// Deletes an installed version on a background thread: a version is thousands of files and
    /// the window must keep drawing meanwhile. `Msg::Removed` ends it.
    fn remove(&mut self, tag: &str) {
        self.confirm_remove = None;
        if self.removing.is_some() || !self.installed.iter().any(|t| t == tag) {
            return;
        }
        self.install_error = None;
        self.removing = Some(tag.to_string());
        self.installed.retain(|t| t != tag);
        self.settings.selected =
            selection_after_removal(&self.installed, self.settings.selected.take());
        let (tag, tx) = (tag.to_string(), self.tx.clone());
        std::thread::spawn(move || {
            let result = backend::remove_version(&tag);
            let _ = tx.send(Msg::Removed(tag, result));
        });
    }

    /// Prepares the selected version from the Minecraft files on this computer.
    fn prepare(&mut self) {
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        let minecraft = self
            .settings
            .minecraft_dir
            .clone()
            .unwrap_or_else(|| horizon_prepare::artifacts::minecraft_dir(None));
        self.prepare_log.clear();
        self.prepare_error = None;
        self.prepare_log
            .push_back("Checking Java and your Minecraft install".into());
        let tx = self.tx.clone();
        // The checks look through every launcher's folders, so they run on the thread too.
        std::thread::spawn(move || {
            let log = |line| {
                let _ = tx.send(Msg::Line(Job::Prepare, line));
            };
            let result = backend::preparer(&tag, &minecraft, &log)
                .inspect(|(_, minecraft)| backend::sounds(&tag, minecraft, &log))
                .and_then(|(p, _)| match p {
                    backend::Preparer::Process(cmd) => {
                        backend::spawn(Job::Prepare, cmd, tx.clone()).map(|_running| false)
                    }
                    backend::Preparer::InProcess(options) => {
                        backend::prepare_in_process(&options, &log).map(|_| true)
                    }
                });
            match result {
                // A process sends its own Exit when it ends.
                Ok(false) => {}
                Ok(true) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Ok(())));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
                }
            }
        });
        self.preparing = true;
        self.prepare_started = Some(Instant::now());
    }

    fn play(&mut self) {
        let Some(tag) = self.selected().cloned() else {
            return;
        };
        if self.client.is_some() || self.starting_client.is_some() {
            return;
        }
        self.start_client(tag);
    }

    /// Starts the game: what the resources lack comes first, then `drain` starts the process.
    fn start_client(&mut self, tag: String) {
        self.client_error = None;
        self.crash = None;
        self.prepare_log.clear();
        // A little earlier than the spawn: file times can be coarse.
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        // What the resources lack (built-in packs, panorama, sounds) comes first; `drain` starts
        // the game when it is done.
        self.starting_client = Some(tag.clone());
        self.play_pressed = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            backend::complete_assets(&tag);
            let _ = tx.send(Msg::Ready);
        });
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.minecraft_dir = Some(path);
                        this.settings.save();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

// ----- Building blocks ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Primary,
    Secondary,
    Ghost,
}

/// The element under the pointer and the one it just left, with when that happened. Hover
/// backgrounds ease in and out from these rather than snapping.
struct HoverState {
    on: Option<(SharedString, Instant)>,
    off: Option<(SharedString, Instant)>,
}

static HOVER: Mutex<HoverState> = Mutex::new(HoverState {
    on: None,
    off: None,
});

fn track_hover(el: Stateful<Div>, id: SharedString) -> Stateful<Div> {
    el.on_hover(move |hovered, window, _| {
        let mut state = HOVER.lock().unwrap();
        let is_on = state.on.as_ref().is_some_and(|(h, _)| *h == id);
        if *hovered && !is_on {
            state.off = state.on.take();
            state.on = Some((id.clone(), Instant::now()));
        } else if !*hovered && is_on {
            state.off = Some((id.clone(), Instant::now()));
            state.on = None;
        }
        window.refresh();
    })
}

/// A layer under an element's content that fades `color` in while it is hovered. `bleed` lets
/// it reach past the element on the left and right, as the hover plate of a list row does.
fn hover_layer(id: &SharedString, color: Hsla, radius: f32, bleed: f32) -> AnyElement {
    let layer = div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(-bleed))
        .right(px(-bleed))
        .rounded(px(radius));
    let state = HOVER.lock().unwrap();
    // Each transition gets its own animation id, from when it started, so it plays once.
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    let key = |dir: &str, at: Instant| {
        ElementId::named_usize(format!("{id}-{dir}"), (at - start).as_micros() as usize)
    };
    let ease = || Animation::new(theme::QUICK).with_easing(ease_out_quint());
    if let Some((_, at)) = state.on.as_ref().filter(|(h, _)| h == id) {
        return layer
            .with_animation(key("in", *at), ease(), move |d, t| {
                d.bg(color.opacity(color.a * t))
            })
            .into_any_element();
    }
    if let Some((_, at)) = state
        .off
        .as_ref()
        .filter(|(h, at)| h == id && at.elapsed() < theme::QUICK)
    {
        return layer
            .with_animation(key("out", *at), ease(), move |d, t| {
                d.bg(color.opacity(color.a * (1. - t)))
            })
            .into_any_element();
    }
    layer.into_any_element()
}

/// Text set in one step of the type scale: headings (heading, title, display) in the pixel face.
fn text(step: theme::Step) -> Div {
    let d = div().text_size(px(step.px)).line_height(px(step.line_px()));
    let d = if step.mono {
        d.font_family(theme::MONO)
    } else if step.px >= theme::HEADING.px {
        d.font_family(theme::PIXEL)
    } else {
        d
    };
    if step.weight >= 500 {
        d.font_weight(FontWeight::MEDIUM)
    } else {
        d
    }
}

/// A release's tag as a title: mono, so its digits read at a glance.
fn version_tag(tag: &str) -> Div {
    div()
        .font_family(theme::MONO)
        .text_size(px(22.))
        .line_height(px(28.))
        .font_weight(FontWeight::MEDIUM)
        .child(tag.trim_start_matches('v').to_string())
}

/// Skyline's mark: a pixel city skyline in the accent, its reflection below and the horizon
/// across (docs/media/brand/icon.svg, on the same 16-unit grid).
fn mark(size: f32, t: &Theme) -> Div {
    const TOWERS: [(f32, f32, f32, f32); 8] = [
        (1.0, 8.0, 2.0, 3.0),
        (3.0, 5.0, 3.0, 6.0),
        (6.0, 7.0, 2.0, 4.0),
        (8.0, 4.0, 2.0, 7.0),
        (10.0, 9.0, 1.0, 2.0),
        (11.0, 6.0, 3.0, 5.0),
        (14.0, 9.0, 1.0, 2.0),
        (4.0, 3.0, 1.0, 2.0),
    ];
    const REFLECTION: [(f32, f32, f32, f32); 7] = [
        (1.0, 12.0, 2.0, 2.0),
        (3.0, 12.0, 3.0, 3.0),
        (6.0, 12.0, 2.0, 2.0),
        (8.0, 12.0, 2.0, 4.0),
        (10.0, 12.0, 1.0, 1.0),
        (11.0, 12.0, 3.0, 3.0),
        (14.0, 12.0, 1.0, 1.0),
    ];
    let u = size / 16.;
    let block = |(x, y, w, h): (f32, f32, f32, f32), color: Hsla| {
        div()
            .absolute()
            .left(px(x * u))
            .top(px(y * u))
            .w(px(w * u))
            .h(px(h * u))
            .bg(color)
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .children(TOWERS.map(|r| block(r, t.accent)))
        .children(REFLECTION.map(|r| block(r, t.accent.opacity(0.36))))
        .child(block((1., 11.0, 14., 1.), t.ink))
}

/// The last thing the player did was press Tab: focused controls show a ring (a click focuses them
/// too, without one).
static KEYBOARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Makes a control reachable with Tab, pressed with Enter or Space, with a ring when the keyboard
/// put it in focus.
fn keyboard(el: Stateful<Div>, t: &Theme) -> Stateful<Div> {
    let (ring, gap) = (t.accent, t.page);
    // A ring with a gap of page colour, so it shows around filled buttons too. GPUI draws nothing
    // for a shadow without blur: half a pixel keeps it crisp.
    let shadow = |color, spread: f32| gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.5),
        spread_radius: px(spread),
    };
    el.tab_index(0).focus(move |s| {
        if KEYBOARD.load(Ordering::Relaxed) {
            s.shadow(vec![shadow(ring, 4.), shadow(gap, 1.5)])
        } else {
            s
        }
    })
}

/// A button: a block with a light bevel on top and a darker lip below, its label in the pixel
/// face. Primary (the one main action) is the red face; Secondary a raised block; Ghost only
/// its label until the pointer is over it.
fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    t: &Theme,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label: SharedString = label.into();
    let label = SharedString::from(tr(&label).to_string());
    let t = *t;
    let hover = match kind {
        Kind::Primary => t.on_face.opacity(0.10),
        Kind::Secondary => t.bevel.opacity(if t.dark { 0.10 } else { 0.35 }),
        Kind::Ghost => t.raised,
    };
    let base = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(theme::CONTROL))
        .px(px(14.))
        .font_family(theme::PIXEL)
        .text_size(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(hover_layer(&id, hover, 0., 0.));
    let base = match kind {
        Kind::Primary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.face_bevel, t.face_lip, theme::LIP))
            .bg(t.face)
            .text_color(t.on_face),
        Kind::Secondary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.button_bevel, t.lip, theme::LIP))
            .bg(t.button)
            .text_color(t.ink),
        Kind::Ghost => base.text_color(t.muted).hover(move |s| s.text_color(t.ink)),
    };
    // The label sits on the block's face, above its lip; pressing pushes it down.
    let base = base.child(
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .pb(px(if kind == Kind::Ghost { 0. } else { 2. }))
            .child(label),
    );
    keyboard(track_hover(base, id), &t).active(|s| s.opacity(theme::PRESSED).top(px(1.)))
}

/// Opens `url` only when it stays on this computer (a server's own web panel, a file): Skyline
/// opens no outside link.
fn link_allowed(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    };
    match scheme {
        "file" => true,
        "http" | "https" => matches!(host, "localhost" | "127.0.0.1" | "::1"),
        _ => false,
    }
}

pub(crate) fn open_url(cx: &mut App, url: &str) {
    if link_allowed(url) {
        cx.open_url(url);
    } else {
        eprintln!("Skyline opens no outside link: {url}");
    }
}

/// Copies to the clipboard, or into `SKYLINE_LAUNCHER_CLIPBOARD` when a test run sets it
/// (`horizon_launcher::desktop_seams`).
fn copy_to_clipboard(cx: &mut App, item: gpui::ClipboardItem) {
    let Some(file) = horizon_launcher::desktop_seams::clipboard_file() else {
        cx.write_to_clipboard(item);
        return;
    };
    let bytes = match item.entries().first() {
        Some(gpui::ClipboardEntry::Image(image)) => image.bytes.clone(),
        _ => item.text().unwrap_or_default().into_bytes(),
    };
    if let Err(e) = std::fs::write(&file, bytes) {
        eprintln!("{}: {e}", file.display());
    }
}

/// An on/off switch: a square knob sliding in a sunk track that turns red when on.
fn switch(id: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<Div> {
    let t = *t;
    let id: SharedString = id.into();
    let knob = if on { t.on_face } else { t.muted };
    keyboard(div().id(id.clone()), &t)
        .relative()
        .flex_none()
        .w(px(44.))
        .h(px(24.))
        .border_2()
        .border_color(t.outline)
        .cursor_pointer()
        .bg(if on { t.face } else { t.well })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .size(px(16.))
                .bg(knob)
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(if on { t.face_lip } else { t.lip }),
                )
                .with_animation(
                    ElementId::named_usize(id, on as usize),
                    Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                    move |d, p| d.left(px(if on { 2. + 20. * p } else { 22. - 20. * p })),
                ),
        )
}

/// A section's title inside a page: a pixel icon in the accent and the name in the pixel face.
fn section_title(label: &'static str, icon: &'static str, t: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(blocks::icon(icon, 20., t.accent))
        .child(
            blocks::pixel(18., FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(tr(label)),
        )
}

/// A segmented control: a sunk track, the chosen segment a raised block.
fn segments<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: T,
    t: &Theme,
    on_pick: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    let t = *t;
    div()
        .id(id)
        .flex()
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .border_2()
        .border_color(t.outline)
        .bg(t.well)
        .children(options.iter().map(|&(value, label)| {
            let active = current == value;
            let pick = on_pick.clone();
            keyboard(div().id(SharedString::from(format!("{id}-{label}"))), &t)
                .relative()
                .flex()
                .items_center()
                .h(px(theme::SEGMENT))
                .px(px(14.))
                .pb(px(2.))
                .font_family(theme::PIXEL)
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.button).text_color(t.ink).children(blocks::edges(
                        t.button_bevel,
                        t.lip,
                        3.,
                    ))
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(move |s| s.text_color(t.ink))
                })
                .child(div().relative().child(tr(label)))
                .on_click(move |_: &ClickEvent, window, cx| pick(value, window, cx))
        }))
}

/// The selected version once one has been removed: the saved choice if it is still installed,
/// else the newest left.
fn selection_after_removal(installed: &[String], saved: Option<String>) -> Option<String> {
    saved
        .filter(|s| installed.contains(s))
        .or_else(|| installed.first().cloned())
}

fn disabled(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(theme::DISABLED).cursor_default().tab_stop(false)
}

/// A page's title in the pixel face and one line on what it is for.
fn page_header(title: &str, description: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            blocks::pixel(36., FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(tr(title).to_string()),
        )
        .child(
            text(theme::UI)
                .max_w(px(520.))
                .text_color(t.muted)
                .child(tr(description).to_string()),
        )
}

/// One row of a list, as its own panel: a name in the pixel face with an optional detail under
/// it, then a value or a control on the right. Rows were numbered in the quiet look; `_n` keeps
/// the call sites as they are.
fn index_row(
    _n: usize,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    slot_row(None, name, detail, right, t)
}

/// A row with an icon slot (`blocks::slot`) on its left: a project in the Mods page's lists.
fn slot_row(
    slot: Option<Div>,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    blocks::panel(t)
        .flex()
        .items_center()
        .gap(px(16.))
        .min_h(px(theme::ROW))
        .mb(px(8.))
        .px(px(16.))
        .py(px(12.))
        .children(slot)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    blocks::pixel(16., FontWeight::MEDIUM)
                        .text_color(t.ink)
                        .child(name),
                )
                .children(detail.map(|d| {
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(d)
                        .into_any_element()
                })),
        )
        .children(right)
}

/// A plate that stands apart from the page: notices, the console, the agents note.
fn plate(t: &Theme) -> Div {
    blocks::panel(t)
}

/// Something went wrong: a dot in the danger colour, then what happened.
fn notice(text_: String, t: &Theme) -> Div {
    plate(t)
        .flex()
        .gap(px(12.))
        .p(px(16.))
        .text_size(px(theme::UI.px))
        .line_height(px(21.))
        .text_color(t.ink)
        .border_l(px(6.))
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .bottom(px(-2.))
                .left(px(-6.))
                .w(px(6.))
                .bg(t.danger),
        )
        .child(blocks::icon("info-box", 20., t.danger))
        .child(div().flex_1().min_w_0().child(text_))
}

/// An empty state: a short sentence, and the one thing to do about it.
fn centered(text_: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(16.))
        .py(px(48.))
        .child(text(theme::UI).text_color(t.muted).child(text_.to_string()))
}

/// The name the game made up for a player without an account, once it has run.
fn offline_name() -> Option<String> {
    player_name(&std::fs::read_to_string(backend::game_home().join("skyline-player.txt")).ok()?)
}

/// The name in `skyline-player.txt`: the game writes `name:Player12345` (horizon-client's
/// offline_name.rs).
fn player_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("name:"))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
}

/// A mono line in upper case: the status above a title, the footer's links.
fn caps(s: impl AsRef<str>) -> Div {
    text(theme::MICRO).child(s.as_ref().to_uppercase())
}

fn short(path: &std::path::Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}

// ----- Pages -------------------------------------------------------------------------------------

impl Launcher {
    /// The top bar: the mark and name over the sidebar (in the title bar on Windows and Linux
    /// client-side decorations), the pages with their icons, then the account and Settings.
    fn topbar(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut nav = div().flex().flex_none().h_full().gap(px(2.));
        for (page, label, icon) in Page::ALL {
            if page == Page::Settings {
                continue;
            }
            let active = self.page == page;
            let id = SharedString::from(format!("nav-{label}"));
            let item = div()
                .id(id.clone())
                .relative()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(14.))
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.raised).child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .right(px(8.))
                            .bottom_0()
                            .h(px(4.))
                            .bg(t.accent),
                    )
                })
                .when(!active, |d| d.child(hover_layer(&id, t.raised, 0., 0.)))
                .child(div().relative().child(blocks::icon(
                    icon,
                    18.,
                    if active { t.accent } else { t.muted },
                )))
                .child(
                    blocks::pixel(16., FontWeight::MEDIUM)
                        .relative()
                        .text_color(if active { t.ink } else { t.muted })
                        .child(tr(label)),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.go(page);
                    cx.notify();
                }));
            nav = nav.child(keyboard(track_hover(item, id), &t));
        }
        // Only a signed-in account says "Microsoft account"; otherwise an offline name, or the
        // one the game made up.
        let (name, how) = self.who_plays();
        let account = keyboard(div().id("account"), &t)
            .relative()
            .flex()
            .min_w_0()
            .max_w(px(180.))
            .items_center()
            .gap(px(10.))
            .h(px(40.))
            .pl(px(4.))
            .pr(px(12.))
            .bg(t.panel)
            .border_2()
            .border_color(t.outline)
            .cursor_pointer()
            .hover(move |s| s.bg(t.raised))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(30.))
                    .bg(t.well)
                    .child(blocks::icon("user", 18., t.muted)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        blocks::pixel(14., FontWeight::MEDIUM)
                            .truncate()
                            .text_color(t.ink)
                            .child(name),
                    )
                    .child(text(theme::CAPTION).text_color(t.muted).child(how)),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.account_menu = !this.account_menu;
                cx.notify();
            }));
        let account = div()
            .relative()
            .child(account)
            .children(self.account_menu(cx));
        let settings_open = self.page == Page::Settings;
        let settings = keyboard(div().id("nav-Settings"), &t)
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(40.))
            .bg(if settings_open { t.raised } else { t.panel })
            .border_2()
            .border_color(t.outline)
            .cursor_pointer()
            .hover(move |s| s.bg(t.raised))
            .when(settings_open, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(4.))
                        .bg(t.accent),
                )
            })
            .child(blocks::icon(
                "sliders",
                20.,
                if settings_open { t.accent } else { t.ink_soft },
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.go(Page::Settings);
                cx.notify();
            }));
        div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(theme::TOPBAR))
            .pl(px(18.))
            .pr(px(12.))
            .bg(t.side)
            .border_b_2()
            .border_color(t.outline)
            // Custom title bars carry the mark and name above the sidebar.
            .when(!titlebar::draws_brand(window), |bar| {
                bar.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(10.))
                        .w(px(theme::SIDEBAR - 18.))
                        .child(mark(24., &t))
                        .child(
                            blocks::pixel(20., FontWeight::SEMIBOLD)
                                .text_color(t.ink)
                                .child("SKYLINE"),
                        ),
                )
            })
            .child(nav)
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .child(account)
                    .child(settings),
            )
    }

    /// The sidebar: Skyline and the imported modpacks, the player's servers, then the
    /// launcher's own update, the community links and its version.
    fn sidebar(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let heading = |label: String| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(6.))
                .pt(px(10.))
                .pb(px(6.))
                .child(
                    blocks::pixel(13., FontWeight::MEDIUM)
                        .text_color(t.muted)
                        .child(label),
                )
        };
        let sub = |s: String| text(theme::CAPTION).text_color(t.muted).truncate().child(s);
        let title = |s: String| {
            blocks::pixel(15., FontWeight::MEDIUM)
                .text_color(t.ink)
                .truncate()
                .child(s)
        };
        let dot = |c: Hsla| {
            div()
                .absolute()
                .left(px(42.))
                .top(px(42.))
                .size(px(10.))
                .bg(c)
                .border_2()
                .border_color(t.side)
        };
        let add = keyboard(div().id("add-instance"), &t)
            .flex()
            .items_center()
            .justify_center()
            .size(px(24.))
            .bg(t.panel)
            .border_2()
            .border_color(t.outline)
            .cursor_pointer()
            .hover(move |s| s.bg(t.raised))
            .child(blocks::icon("plus", 14., t.ink_soft))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_instance_dialog(None);
                cx.notify();
            }));
        let items = self.instance_items(heading, cx);
        let list = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(heading(tr("INSTANCES").into()).child(add))
            .children(items);
        let servers: Vec<_> = self
            .servers
            .sidebar_list()
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| self.side_servers.clone());
        let mut list = list;
        if !servers.is_empty() {
            list = list.child(heading(tr("SERVERS").into()));
        }
        for e in servers {
            let up = self.servers.is_up(&e.dir);
            let open = self.page == Page::Servers && self.servers.open.as_ref() == Some(&e.dir);
            let dir = e.dir.clone();
            list = list.child(
                blocks::side_item(format!("side-server-{}", e.dir.display()).into(), open, &t)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(44.))
                            .bg(t.well)
                            .border_2()
                            .border_color(t.outline)
                            .child(blocks::icon("server", 22., t.ink_soft)),
                    )
                    .child(dot(if up { t.success } else { t.muted }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(title(e.name.clone()))
                            .child(sub(tr(if up { "Running" } else { "Stopped" }).into())),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.servers.open = Some(dir.clone());
                        this.go(Page::Servers);
                        cx.notify();
                    })),
            );
        }
        let foot = div()
            .mt_auto()
            .flex()
            .flex_col()
            .gap(px(2.))
            .pt(px(12.))
            .child(
                caps(format!("Skyline {}  ·  offline", backend::VERSION))
                    .px(px(8.))
                    .pt(px(6.))
                    .text_color(t.muted),
            );
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(theme::SIDEBAR))
            .h_full()
            .px(px(10.))
            .pb(px(12.))
            .bg(t.side)
            .border_r_2()
            .border_color(t.outline)
            .overflow_y_scroll()
            .child(list)
            .child(foot)
    }

    fn picker(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let label = self
            .selected()
            .cloned()
            .unwrap_or_else(|| "No version".into());
        // A block as tall as Play: "VERSION" over the tag, and a chevron.
        let mut wrap = div().relative().child(
            keyboard(div().id("picker"), &t)
                .relative()
                .flex()
                .items_center()
                .gap(px(16.))
                .h(px(60.))
                .pl(px(14.))
                .pr(px(12.))
                .bg(t.panel)
                .border_2()
                .border_color(t.outline)
                .children(blocks::edges(t.bevel, t.lip, theme::BEVEL))
                .cursor_pointer()
                .hover(move |s| s.bg(t.raised))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            blocks::pixel(11., FontWeight::MEDIUM)
                                .text_color(t.muted)
                                .child("VERSION"),
                        )
                        .child(
                            div()
                                .font_family(theme::MONO)
                                .text_size(px(15.))
                                .text_color(t.ink)
                                .child(label),
                        ),
                )
                .child(blocks::icon("chevron-down", 20., t.muted))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.picker_open = !this.picker_open;
                    cx.notify();
                })),
        );
        if self.picker_open {
            let mut menu = div()
                .absolute()
                .top(px(66.))
                .right_0()
                .w(px(300.))
                .p(px(6.))
                .border_2()
                .border_color(t.outline)
                .bg(t.surface)
                .flex()
                .flex_col();
            // Picking a version keeps the selected instance on it; the launcher's own version
            // (newest unless chosen otherwise on Versions) follows updates.
            let follows = self.launcher_version().cloned();
            for tag in &self.installed {
                let selected = self.selected() == Some(tag);
                let tag = tag.clone();
                let latest = follows.as_ref() == Some(&tag);
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("pick-{tag}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(px(theme::RADIUS))
                        .font_family(theme::MONO)
                        .text_size(px(theme::CAPTION.px))
                        .text_color(t.ink)
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.plate_soft))
                        .child(tag.clone())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .when(latest, |d| {
                                    d.child(caps("follows updates").text_color(t.muted))
                                })
                                .when(selected, |d| {
                                    d.child(div().size(px(6.)).rounded_full().bg(t.accent))
                                }),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pin_version((!latest).then(|| tag.clone()));
                            this.picker_open = false;
                            cx.notify();
                        })),
                );
            }
            menu = menu.child(div().my(px(6.)).h(px(1.)).bg(t.rule)).child(
                div()
                    .id("manage")
                    .flex()
                    .items_center()
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(theme::RADIUS))
                    .text_size(px(theme::LABEL.px))
                    .text_color(t.muted)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.plate_soft).text_color(t.ink))
                    .child("Manage versions")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.go(Page::Versions);
                        this.picker_open = false;
                        cx.notify();
                    })),
            );
            wrap = wrap.child(gpui::deferred(menu).with_priority(1));
        }
        wrap
    }

    fn play_page(&mut self, banner_width: f32, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let Some(tag) = self.selected().cloned() else {
            // No hero without a version: the page starts below the top bar like the others.
            return div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .pt(px(28.))
                .child(page_header(
                    "Play",
                    "Start the game, or run a server for others to join.",
                    &t,
                ))
                .child(centered(tr("No version installed yet."), &t).child(
                    button("go-versions", "Choose a version", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go(Page::Versions);
                            cx.notify();
                        }),
                    ),
                ));
        };
        let prepared = backend::is_prepared(&tag);
        // Prepared without sounds (#1035): Prepare again is the fix, so it stands out.
        let silent = prepared && !backend::has_sounds(&tag);
        let version = backend::game_version(&tag);
        let hero = self.hero(&tag, &version, prepared, cx);
        let running = self.client.is_some() || self.starting_client.is_some();
        let play_label = if running { "Running" } else { "Play" };
        // The label clears the icon in every language ("Играть" is wider than "Play").
        let mut play = button("play", play_label, Kind::Primary, &t)
            .min_w(px(200.))
            .h(px(60.))
            .pl(px(60.))
            .pr(px(28.))
            .text_size(px(28.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(
                div()
                    .absolute()
                    .left(px(26.))
                    .top(px(17.))
                    .child(blocks::icon("play", 22., t.on_face)),
            );
        if !prepared || self.preparing || running {
            play = disabled(play);
        } else {
            play = play.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.picker_open = false;
                this.play();
                cx.notify();
            }));
        }
        let actions = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                button("host", "Host a server", Kind::Ghost, &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.go(Page::Servers);
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.picker(cx))
                    .child(play),
            );

        let resources_value = if self.preparing {
            tr("Preparing…").to_string()
        } else {
            backend::resources_summary(prepared, !silent, &version)
        };
        let mut prepare = button(
            "prepare",
            if prepared { "Prepare again" } else { "Prepare" },
            if prepared && !silent {
                Kind::Ghost
            } else {
                Kind::Secondary
            },
            &t,
        );
        if self.preparing {
            prepare = disabled(prepare);
        } else {
            prepare = prepare.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.prepare();
                cx.notify();
            }));
        }
        let prepare = div().child(prepare);
        let minecraft = self.settings.minecraft_dir.as_deref().map_or_else(
            || {
                div()
                    .child(tr(
                        "Not found. Choose the folder where your launcher keeps Minecraft.",
                    ))
                    .into_any_element()
            },
            |p| {
                div()
                    .font_family(theme::MONO)
                    .child(short(p))
                    .into_any_element()
            },
        );
        let rows = div()
            .flex()
            .flex_col()
            .child(section_title("Ready to play", "check", &t).mb(px(12.)))
            .children(self.instance_row(1, cx))
            .children(self.export_panel(cx))
            .children(self.launch_panel(cx))
            .child(index_row(
                2,
                tr("Resources"),
                Some(div().child(resources_value).into_any_element()),
                Some(prepare.into_any_element()),
                &t,
            ))
            .child(index_row(
                3,
                tr("Minecraft folder"),
                Some(minecraft),
                Some(
                    button("choose-mc", "Choose…", Kind::Ghost, &t)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(cx)))
                        .into_any_element(),
                ),
                &t,
            ));

        // The banner: Skyline's landscape (`art.rs`) under a veil of the page colour, the hero and
        // the Play row over it. It reaches the content's edges and the top bar.
        let (scene, scene_width) = self.art.scene(t.dark, self.scale, banner_width);
        let veil = |angle: f32, from: f32, to: f32, at: f32| {
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(gpui::linear_gradient(
                    angle,
                    gpui::linear_color_stop(t.page.opacity(from), 0.),
                    gpui::linear_color_stop(t.page.opacity(to), at),
                ))
        };
        let picture = self.home_background();
        let scene = match picture {
            Some(path) => gpui::img(path)
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .object_fit(gpui::ObjectFit::Cover),
            None => gpui::img(scene)
                .absolute()
                .bottom_0()
                .left_0()
                .w(px(scene_width))
                .h(px(art::SCENE_HEIGHT as f32 * art::SCENE_TEXEL)),
        };
        let banner = div()
            .relative()
            .flex()
            .flex_col()
            .justify_end()
            .min_h(px(340.))
            .mx(px(-theme::PAGE_GUTTER))
            .px(px(theme::PAGE_GUTTER))
            .pb(px(28.))
            .gap(px(24.))
            .overflow_hidden()
            .bg(gpui::rgb(art::sky_top(t.dark)))
            .border_b_2()
            .border_color(t.outline)
            .child(scene)
            .child(veil(90., 0.82, 0., 0.62))
            .child(veil(180., 0., 0.75, 1.))
            .child(div().relative().child(hero))
            .child(div().relative().child(actions));
        let mut page = div().flex().flex_col().gap(px(28.)).child(banner);
        // The preparation's progress and error right under the buttons, where the player
        // looks after pressing Prepare, not below the fold (#666).
        if self.preparing {
            let last = self.prepare_log.back().cloned().unwrap_or_default();
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(progress_bar(None, &t)),
                    )
                    .child(
                        text(theme::MICRO)
                            .text_color(t.muted)
                            .truncate()
                            .child(last),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child(tr("The first preparation takes a few minutes.")),
                    ),
            );
        }
        if let Some(e) = self.prepare_error.clone() {
            page = page.child(notice(e, &t));
        }
        page = page.child(rows);
        if let Some(console) = self.console_section(cx) {
            page = page.child(console);
        }
        if let Some(worlds) = self.worlds_section(cx) {
            page = page.child(worlds);
        }
        if let Some(servers) = self.multiplayer_section(cx) {
            page = page.child(servers);
        }
        if let Some(shots) = self.screenshots_section(cx) {
            page = page.child(shots);
        }
        if let Some(e) = self.client_error.clone() {
            page = page.child(notice(e, &t));
        }
        if let Some(crash) = self.crash.clone() {
            page = page.child(self.crash_notice(crash, cx));
        }
        page
    }

    /// "Skyline crashed": the report and the logs, and where to report it.
    fn crash_notice(&self, crash: Crash, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut actions = div().flex().flex_wrap().gap(px(8.)).mt(px(8.));
        if let Some(report) = crash.report.clone() {
            let open = report.clone();
            actions = actions
                .child(
                    button("crash-open", "Open report", Kind::Secondary, &t)
                        .bg(t.plate)
                        .on_click(move |_, _, cx| cx.open_with_system(&open)),
                )
                .child(
                    button(
                        "crash-copy",
                        if crash.copied {
                            "Copied"
                        } else {
                            "Copy report"
                        },
                        Kind::Secondary,
                        &t,
                    )
                    .bg(t.plate)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Ok(text) = std::fs::read_to_string(&report) {
                                copy_to_clipboard(cx, gpui::ClipboardItem::new_string(text));
                                if let Some(c) = &mut this.crash {
                                    c.copied = true;
                                }
                            }
                            cx.notify();
                        },
                    )),
                );
        }
        actions = actions.child(
            button("crash-logs", "Open logs folder", Kind::Secondary, &t)
                .bg(t.plate)
                .on_click(|_, _, cx| {
                    let dir = backend::logs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    cx.open_with_system(&dir);
                }),
        );
        actions = actions.child(
            button("crash-dismiss", "Dismiss", Kind::Ghost, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.crash = None;
                    cx.notify();
                },
            )),
        );
        let help = tr(if crash.report.is_some() {
            "The crash report says what went wrong. Your worlds are saved up to the last autosave."
        } else {
            "The game ended without a crash report: latest.log in the logs folder shows its last lines."
        });
        let mut body = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child(tr("Skyline crashed")),
            );
        if !crash.summary.is_empty() {
            body = body.child(div().text_color(t.ink_soft).child(crash.summary));
        }
        if let Some(report) = &crash.report {
            body = body.child(
                text(theme::MICRO)
                    .text_color(t.muted)
                    .truncate()
                    .child(short(report)),
            );
        }
        body = body.child(actions).child(
            text(theme::CAPTION)
                .mt(px(4.))
                .text_color(t.muted)
                .child(help),
        );
        plate(&t)
            .flex()
            .gap(px(14.))
            .p(px(24.))
            .text_size(px(theme::UI.px))
            .line_height(px(21.))
            .text_color(t.ink)
            .child(
                div()
                    .flex_none()
                    .mt(px(11.))
                    .size(px(6.))
                    .rounded_full()
                    .bg(t.danger),
            )
            .child(body)
    }

    /// The Play page's focal point: Skyline, the instance that plays and whether it is ready.
    fn hero(&self, _tag: &str, version: &str, prepared: bool, _cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let instance = self.selected_instance().map(|i| i.name.clone());
        div()
            .flex()
            .flex_none()
            .flex_col()
            .gap(px(12.))
            .child(
                div().flex().child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py(px(5.))
                        .bg(t.side.opacity(0.85))
                        .border_2()
                        .border_color(t.outline)
                        .child(
                            div()
                                .size(px(8.))
                                .bg(if prepared { t.success } else { t.warn }),
                        )
                        .child(caps(format!("Minecraft {version}")).text_color(t.ink_soft))
                        .child(div().size(px(4.)).bg(t.muted))
                        .child(
                            caps(tr(if prepared { "Ready" } else { "Needs preparing" }))
                                .text_color(if prepared { t.success } else { t.warn }),
                        ),
                ),
            )
            .child(
                blocks::pixel(60., FontWeight::BOLD)
                    .line_height(px(62.))
                    .text_color(t.ink)
                    .child("Skyline"),
            )
            .children(instance.map(|name| {
                text(theme::BODY)
                    .max_w(px(560.))
                    .text_color(t.ink_soft)
                    .child(name)
            }))
    }

    fn settings_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let opening = switch("opening", !self.settings.skip_opening, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.skip_opening = !this.settings.skip_opening;
                this.settings.save();
                cx.notify();
            },
        ));
        // Linux: Skyline's entry in the applications menu (`horizon_launcher::desktop`).
        let menu_entry = switch("desktop-entry", self.settings.desktop_entry(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                let on = !this.settings.desktop_entry();
                this.settings.desktop_entry = Some(on);
                this.settings.save();
                let _ = if on {
                    horizon_launcher::desktop::install(backend::exe_path()).map(|_| ())
                } else {
                    horizon_launcher::desktop::remove()
                };
                cx.notify();
            }),
        );
        let mono = |s: String| div().font_family(theme::MONO).child(s).into_any_element();
        let plain = |s: &str| div().child(tr(s).to_string()).into_any_element();
        let (offline, n) = self.offline_rows(0, cx);
        let appearance = segments(
            "appearance",
            &[
                (backend::Appearance::Dark, "Dark"),
                (backend::Appearance::Light, "Light"),
                (backend::Appearance::System, "System"),
            ],
            self.settings.appearance,
            &t,
            {
                let this = cx.entity().downgrade();
                move |choice, window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.appearance = choice;
                        this.settings.save();
                        titlebar::apply_appearance(window, choice);
                        this.theme = this.settings_theme(window);
                        cx.notify();
                    });
                }
            },
        );
        let group = |label: &'static str, icon: &'static str| {
            section_title(label, icon, &t).mt(px(20.)).mb(px(12.))
        };
        let open_button = |id: &'static str, dir: std::path::PathBuf| {
            button(id, "Open", Kind::Secondary, &t)
                .on_click(move |_, _, cx| {
                    let _ = std::fs::create_dir_all(&dir);
                    cx.open_with_system(&dir);
                })
                .into_any_element()
        };
        let build = match option_env!("SKYLINE_COMMIT") {
            Some(commit) => format!("{}  ·  {commit}", backend::VERSION),
            None => backend::VERSION.to_string(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(32.))
            .child(page_header(
                "Settings",
                "Who plays, how Skyline looks, and where it keeps your things.",
                &t,
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(group("Player", "user").mt_0())
                    .child(offline)
                    .child(group("Appearance", "monitor"))
                    .child(index_row(
                        n + 1,
                        tr("Theme"),
                        Some(plain("Dark by default. System follows your computer's light or dark.")),
                        Some(appearance.into_any_element()),
                        &t,
                    ))
                    .child(self.accent_row(cx))
                    .child(self.home_background_row(cx))
                    .child(self.language_row(cx))
                    .child(index_row(
                        n + 2,
                        tr("Show the opening"),
                        Some(plain("The Skyline wordmark when the window opens.")),
                        Some(opening.into_any_element()),
                        &t,
                    ))
                    .child(group("Game", "gamepad"))
                    .child(self.game_args_row(n + 3, cx))
                    .child(self.while_playing_row(n + 4, cx))
                    .child(group("About", "info-box"))
                    .child(index_row(
                        n + 5,
                        "Skyline 26.3",
                        Some(div().flex().flex_col().child(tr("Offline edition: no accounts, no updates, nothing leaves this computer.").to_string()).child(mono(build)).into_any_element()),
                        None,
                        &t,
                    ))
                    .child(index_row(
                        n + 6,
                        tr("Data folder"),
                        Some(mono(short(&backend::data_dir()))),
                        Some(open_button("open-data", backend::data_dir())),
                        &t,
                    ))
                    .child(index_row(
                        n + 7,
                        tr("Game folder"),
                        Some(mono(short(&backend::game_home()))),
                        Some(open_button("open-game", backend::game_home())),
                        &t,
                    ))
                    .child(index_row(
                        n + 8,
                        tr("Logs"),
                        Some(mono(format!(
                            "{}  ·  {}",
                            short(&backend::logs_dir()),
                            tr("crash reports beside it")
                        ))),
                        Some(open_button("open-logs", backend::logs_dir())),
                        &t,
                    ))
                    .when(cfg!(target_os = "linux"), |d| {
                        d.child(index_row(
                            n + 9,
                            tr("Applications menu"),
                            Some(plain(if self.settings.desktop_entry() {
                                "Skyline and its icon are in your desktop's menu."
                            } else {
                                "Not in your desktop's menu."
                            })),
                            Some(menu_entry.into_any_element()),
                            &t,
                        ))
                    }),
            )
    }

    /// Settings' Game arguments row: the extra arguments the game starts with, kept in
    /// `settings.json` and appended to the launcher's own (quotes group a word with spaces).
    fn game_args_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.args_focused;
        let field = div()
            .id("game-args")
            .track_focus(&self.args_focus)
            .flex()
            .items_center()
            .w(px(300.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.args_input,
                &t,
                focused,
                false,
                false,
                "For example --server-thread",
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.args_focus);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.args_input, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => window.blur(),
                    line_edit::Key::Edited => {
                        this.settings.game_args = this.args_input.text().to_string();
                        this.settings.save();
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.args_input);
        index_row(
            n,
            "Game arguments",
            Some(
                div()
                    .child("Extra arguments for the game, passed through as-is (quotes group a word with spaces). Applies the next time the game starts.")
                    .into_any_element(),
            ),
            Some(field.into_any_element()),
            &t,
        )
    }

    /// Plays the selected instance straight into a world (`--world`) or a server (`--server`):
    /// flags every Skyline release takes.
    pub(crate) fn quick_play(&mut self, flag: &'static str, value: String) {
        self.quick_play = Some((flag, value));
        self.play();
        if self.starting_client.is_none() {
            self.quick_play = None;
        }
    }

    /// Versions' Use: the version Play starts, for the launcher and for the selected instance
    /// when it keeps a version of its own.
    fn use_version(&mut self, tag: &str) {
        self.settings.selected = Some(tag.to_string());
        self.settings.save();
        if self
            .selected_instance()
            .is_some_and(|i| i.version.is_some())
        {
            self.pin_version(Some(tag.to_string()));
        }
    }

    /// Versions' Add from folder: a Skyline build already on this computer becomes a version.
    fn add_version(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        match backend::add_version_from_folder(&path) {
                            Ok(tag) => {
                                this.installed = backend::installed();
                                this.install_error = None;
                                if this.settings.selected.is_none() {
                                    this.settings.selected = Some(tag);
                                    this.settings.save();
                                }
                            }
                            Err(e) => this.install_error = Some((String::new(), e)),
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    /// The versions on this computer: each one's game version and whether it is prepared, with
    /// Use, Prepare, Open folder and Remove.
    fn versions_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let header = div()
            .flex()
            .items_end()
            .justify_between()
            .gap(px(16.))
            .child(page_header(
                "Versions",
                "The Skyline builds on this computer. Each one keeps its own prepared Minecraft files.",
                &t,
            ))
            .child(
                button("add-version", "Add from folder…", Kind::Secondary, &t).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.add_version(cx);
                        cx.notify();
                    }),
                ),
            );
        let mut page = div().flex().flex_col().gap(px(24.)).child(header);
        if let Some((tag, e)) = self.install_error.clone().filter(|(t, _)| t.is_empty()) {
            let _ = tag;
            page = page.child(notice(e, &t));
        }
        if self.installed.is_empty() && self.removing.is_none() {
            return page.child(centered(
                tr("No version yet. Build Skyline (cargo build --release) and add its folder."),
                &t,
            ));
        }
        let mut list = div().flex().flex_col();
        let mut tags = self.installed.clone();
        tags.extend(self.removing.clone());
        for tag in tags {
            let removing = self.removing.as_ref() == Some(&tag);
            let selected = self.selected() == Some(&tag);
            let prepared = backend::is_prepared(&tag);
            let runs = backend::has_program(&tag);
            let version = backend::game_version(&tag);
            let confirming = self.confirm_remove.as_ref() == Some(&tag);
            let state = if !runs {
                tr("Made for an older launcher: it has no skyline program. Remove it.").to_string()
            } else if prepared {
                trf("Minecraft {version} · prepared", &[("version", &version)])
            } else {
                trf(
                    "Minecraft {version} · not prepared yet",
                    &[("version", &version)],
                )
            };
            let right: AnyElement = if removing {
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .w(px(180.))
                    .child(progress_bar(None, &t))
                    .child(
                        text(theme::MICRO)
                            .text_color(t.muted)
                            .child(tr("Removing…")),
                    )
                    .into_any_element()
            } else {
                let (use_tag, prep_tag, rm_tag) = (tag.clone(), tag.clone(), tag.clone());
                let folder = backend::version_dir(&tag);
                let mut row = div().flex().items_center().gap(px(4.));
                if !selected && runs && !confirming {
                    row = row.child(
                        button(format!("use-{tag}"), "Use", Kind::Secondary, &t).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.use_version(&use_tag);
                                cx.notify();
                            }),
                        ),
                    );
                }
                if runs && !confirming {
                    let mut prepare = button(
                        format!("prepare-{tag}"),
                        if prepared { "Prepare again" } else { "Prepare" },
                        Kind::Ghost,
                        &t,
                    );
                    if self.preparing || self.client.is_some() {
                        prepare = disabled(prepare);
                    } else {
                        prepare =
                            prepare.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.use_version(&prep_tag);
                                this.prepare();
                                this.go(Page::Play);
                                cx.notify();
                            }));
                    }
                    row = row.child(prepare);
                }
                if !confirming {
                    row = row.child(
                        button(format!("folder-{tag}"), "Open folder", Kind::Ghost, &t)
                            .on_click(move |_, _, cx| cx.open_with_system(&folder)),
                    );
                }
                row.child(
                    button(
                        format!("remove-{tag}"),
                        if confirming {
                            "Remove for good"
                        } else {
                            "Remove"
                        },
                        if confirming {
                            Kind::Secondary
                        } else {
                            Kind::Ghost
                        },
                        &t,
                    )
                    .when(confirming, |b| b.text_color(t.danger))
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if this.confirm_remove.as_ref() == Some(&rm_tag) {
                                this.remove(&rm_tag);
                            } else {
                                this.confirm_remove = Some(rm_tag.clone());
                            }
                            cx.notify();
                        },
                    )),
                )
                .when(confirming, |d| {
                    d.child(
                        button(format!("keep-{tag}"), "Keep", Kind::Ghost, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.confirm_remove = None;
                                cx.notify();
                            }),
                        ),
                    )
                })
                .into_any_element()
            };
            let status = selected.then(|| caps(tr("In use")).text_color(t.accent));
            let error = self
                .install_error
                .as_ref()
                .filter(|(t, _)| *t == tag)
                .map(|(_, e)| e.clone());
            let id = SharedString::from(format!("version-{tag}"));
            let row = div()
                .id(id.clone())
                .relative()
                .flex()
                .flex_col()
                .gap(px(10.))
                .py(px(14.))
                .px(px(16.))
                .mb(px(8.))
                .bg(t.panel)
                .border_2()
                .border_color(t.outline)
                .children(blocks::edges(t.bevel, t.lip, theme::BEVEL))
                .child(hover_layer(&id, t.raised, 0., 0.))
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(12.))
                                        .child(version_tag(&tag).text_color(t.ink))
                                        .children(status),
                                )
                                .child(
                                    text(theme::UI)
                                        .text_color(if runs { t.muted } else { t.warn })
                                        .truncate()
                                        .child(state),
                                ),
                        )
                        .child(div().flex().justify_end().flex_none().child(right)),
                )
                .children(error.map(|e| notice(e, &t)));
            list = list.child(track_hover(row, id));
        }
        page.child(list)
    }
}

/// A thin bar. A known fraction eases from its previous value; an unknown one shuttles slowly.
/// Progress: a sunk track filling with the accent; work of unknown length shuttles a block.
fn progress_bar(fraction: Option<(f32, f32)>, t: &Theme) -> Div {
    let bar = div()
        .absolute()
        .top_0()
        .bottom_0()
        .bg(t.accent)
        .border_b_2()
        .border_color(t.face_lip);
    let track = div()
        .relative()
        .flex_1()
        .h(px(12.))
        .border_2()
        .border_color(t.outline)
        .bg(t.well)
        .overflow_hidden();
    match fraction {
        Some((from, to)) => {
            let (from, to) = (from.clamp(0.02, 1.), to.clamp(0.02, 1.));
            track.child(bar.left_0().with_animation(
                ElementId::named_usize("bar", (to * 1000.) as usize),
                Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                move |d, p| d.w(relative(from + (to - from) * p)),
            ))
        }
        None => track.child(
            bar.w(relative(0.3)).with_animation(
                "shuttle",
                Animation::new(Duration::from_millis(1600))
                    .repeat()
                    .with_easing(gpui::ease_in_out),
                |d, p| d.left(relative(-0.3 + 1.3 * p)),
            ),
        ),
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        self.scale = window.scale_factor();
        self.perf.frame();
        if self.bench.as_ref().is_some_and(perf::Bench::scrolling) {
            self.bench_scroll(window);
        }
        self.servers.note_focus(window);
        self.args_focused = self.args_focus.is_focused(window);
        let size = window.viewport_size();
        let banner_width = f32::from(size.width) - theme::SIDEBAR - scrollbar::GUTTER;
        let content = match self.page {
            Page::Play => self.play_page(banner_width, cx),
            Page::Versions => self.versions_page(cx),
            Page::Servers => self.servers_page(cx),
            Page::Settings => self.settings_page(cx),
        };
        self.art.free_replaced(window);
        // The new page fades in and rises a few pixels.
        let content = div()
            .relative()
            .w_full()
            // Let the page shrink inside the padded flex row so long notices can wrap.
            .min_w_0()
            .child(content)
            .with_animation(
                ElementId::named_usize("page", self.switches),
                Animation::new(theme::ENTER).with_easing(ease_out_quint()),
                |d, p| d.opacity(p).top(px(theme::RISE * (1. - p))),
            );
        let (reveal, held) = self.opening_phase(window);
        let ui = ease_out_cubic((reveal - 0.3) / 0.7);
        if self.opening.is_none() && window.focused(cx).is_none() {
            window.focus(&self.root_focus);
        }
        let stone = self.art.stone(t.page_rgb, t.dark, window.scale_factor());
        let topbar = self.topbar(window, cx);
        let sidebar = self.sidebar(cx);
        let mut root = div()
            .id("root")
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                match k.key.as_str() {
                    "tab" => {
                        KEYBOARD.store(true, Ordering::Relaxed);
                        if k.modifiers.shift {
                            window.focus_prev();
                        } else {
                            window.focus_next();
                        }
                    }
                    "escape" if this.servers_escape() => {}
                    "escape" if this.instance_escape() => {}
                    "escape" if this.screenshot_escape() => {}
                    "left" if this.screenshot_step(false) => {}
                    "right" if this.screenshot_step(true) => {}
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .capture_any_mouse_down(|_, _, _| KEYBOARD.store(false, Ordering::Relaxed))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.page)
            .font_family(theme::FONT)
            .text_color(t.ink)
            .overflow_hidden()
            .child(blocks::stone(stone, size.width.into(), size.height.into()))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .size_full()
                    .opacity(ui)
                    .top(px(theme::RISE * (1. - ui)))
                    .child(topbar)
                    .child(
                        div().flex().flex_1().min_h_0().child(sidebar).child(
                            self.content_scroll
                                .wrap(
                                    div()
                                        .id("content")
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .overflow_y_scroll()
                                        .child(
                                            div()
                                                .flex()
                                                .w_full()
                                                .min_w_0()
                                                .px(px(theme::PAGE_GUTTER))
                                                .pt(px(if self.page == Page::Play {
                                                    0.
                                                } else {
                                                    28.
                                                }))
                                                .pb(px(48.))
                                                .child(content),
                                        ),
                                    &t,
                                )
                                .flex_1()
                                .min_w_0(),
                        ),
                    ),
            );
        if self.page == Page::Servers {
            if let Some(dialog) = self.create_dialog(cx) {
                root = root.child(dialog);
            }
        }
        root = self.import_drop_target(root, cx);
        root = root.children(self.import_note(cx));
        if let Some(dialog) = self.instance_dialog(cx) {
            root = root.child(dialog);
        }
        if self.page == Page::Play {
            if let Some(viewer) = self.screenshot_viewer(cx) {
                root = root.child(viewer);
            }
        }
        if let Some(opening) = &self.opening {
            // The wordmark arrives letter by letter, then gives way to the launcher; a click or
            // a key skips ahead.
            let gone = 1. - ease_out_cubic(reveal / 0.6);
            let word = div()
                .flex()
                .font_family(theme::PIXEL)
                .text_size(px(theme::WORDMARK.px * 0.5))
                .line_height(px(theme::WORDMARK.px * 0.5))
                .font_weight(FontWeight::BOLD)
                .text_color(t.ink)
                .children("SKYLINE".chars().enumerate().map(|(i, c)| {
                    let at = held - i as f32 * theme::STAGGER.as_secs_f32() * 1.5;
                    let p = ease_out_cubic(at / theme::ENTER.as_secs_f32());
                    div()
                        .relative()
                        .opacity(p)
                        .top(px(theme::RISE * 2. * (1. - p)))
                        .child(c.to_string())
                }));
            root = root.child(
                div()
                    .id("opening")
                    .track_focus(&opening.focus)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.page.opacity(gone))
                    .when(reveal == 0., |d| d.cursor_pointer())
                    .child(
                        div()
                            .relative()
                            .top(px(-24. * (1. - gone)))
                            .opacity(gone)
                            .child(word),
                    )
                    .when(reveal == 0., |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.skip_opening();
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(
                            |this, _: &KeyDownEvent, _, cx| {
                                this.skip_opening();
                                cx.notify();
                            },
                        ))
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(t.side)
            .children(titlebar::render(&t, window))
            .child(root.flex_1().min_h_0())
            .children(titlebar::resize_handles(window))
    }
}

impl Launcher {
    /// Where the opening is: the reveal's progress (1 when there is no opening) and how long
    /// the wordmark has been showing, in seconds.
    fn opening_phase(&mut self, window: &mut Window) -> (f32, f32) {
        let loaded = true;
        let Some(opening) = &mut self.opening else {
            return (1., 10.);
        };
        let held = opening.started.get_or_insert_with(Instant::now).elapsed();
        if opening.reveal.is_none() && ((held >= OPENING_HOLD && loaded) || held >= OPENING_MAX) {
            opening.reveal = Some(Instant::now());
        }
        let reveal = opening.reveal.map_or(0., |at| {
            at.elapsed().as_secs_f32() / OPENING_REVEAL.as_secs_f32()
        });
        if reveal >= 1. {
            self.opening = None;
            return (1., 10.);
        }
        window.request_animation_frame();
        (reveal, held.as_secs_f32())
    }

    /// `skyline-launcher benchmark`: scroll the page a little on every frame, back to the top at
    /// the bottom, so the frame times measure scrolling.
    fn bench_scroll(&mut self, window: &mut Window) {
        let handle = &self.content_scroll.handle;
        let mut offset = handle.offset();
        offset.y -= px(24.);
        if -offset.y > handle.max_offset().height {
            offset.y = px(0.);
        }
        handle.set_offset(offset);
        window.request_animation_frame();
    }

    fn skip_opening(&mut self) {
        if let Some(opening) = &mut self.opening {
            opening.reveal.get_or_insert_with(Instant::now);
        }
    }
}

actions!(horizon_launcher, [Quit, CloseWindow]);

/// What closing the last window does: stop the servers this launcher started, send the telemetry,
/// exit. A game that is running keeps running. Cmd+Q goes through here too.
fn shut_down(cx: &mut App) {
    servers_page::stop_on_quit();
    cx.quit()
}

/// The macOS shortcuts: Cmd+Q quits, Cmd+W closes the window.
#[cfg(target_os = "macos")]
fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]
}

#[cfg(not(target_os = "macos"))]
fn key_bindings() -> Vec<KeyBinding> {
    Vec::new()
}

/// The menu bar on macOS, where "Quit" belongs to the app menu.
fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Skyline Launcher".into(),
            items: vec![MenuItem::action("Quit Skyline Launcher", Quit)],
        },
        Menu {
            name: "Window".into(),
            items: vec![MenuItem::action("Close Window", CloseWindow)],
        },
    ]
}

fn main() {
    if let Err(error) = console::attach_for_cli() {
        eprintln!("Could not restore the launcher's terminal streams: {error}");
        std::process::exit(1);
    }
    perf::process_started();
    backend::exe_path();
    if let Some(code) = cli::run() {
        std::process::exit(code);
    }
    Application::new()
        .with_assets(blocks::Assets)
        .run(|cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(horizon_theme::fonts::GEIST_REGULAR),
                    Cow::Borrowed(horizon_theme::fonts::GEIST_MEDIUM),
                    Cow::Borrowed(horizon_theme::fonts::GEIST_SEMIBOLD),
                    Cow::Borrowed(horizon_theme::fonts::GEIST_MONO),
                    Cow::Borrowed(horizon_theme::fonts::PIXEL_MEDIUM),
                    Cow::Borrowed(horizon_theme::fonts::PIXEL_SEMIBOLD),
                    Cow::Borrowed(horizon_theme::fonts::PIXEL_BOLD),
                ])
                .expect("bundled fonts");
            cx.bind_keys(key_bindings());
            cx.on_action(|_: &Quit, cx| shut_down(cx));
            // The key press is being handled inside the window, so the window can only be
            // removed once that update has finished.
            cx.on_action(|_: &CloseWindow, cx| {
                cx.defer(|cx| {
                    for window in cx.windows() {
                        let _ = window.update(cx, |_, window, _| window.remove_window());
                    }
                })
            });
            cx.set_menus(menus());
            let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(960.), px(600.))),
                    titlebar: Some(titlebar::options()),
                    #[cfg(target_os = "linux")]
                    window_decorations: Some(gpui::WindowDecorations::Client),
                    app_id: Some("skyline-launcher".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Launcher::new(window, cx)),
            )
            .expect("window");
            cx.on_window_closed(shut_down).detach();
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn cmd_q_quits_and_cmd_w_closes_the_window() {
        use super::{key_bindings, CloseWindow, Quit};
        use gpui::{Keymap, Keystroke};
        let keymap = Keymap::new(key_bindings());
        let key = |s: &str| [Keystroke::parse(s).unwrap()];
        let (hit, _) = keymap.bindings_for_input(&key("cmd-q"), &[]);
        assert!(hit.iter().any(|b| b.action().partial_eq(&Quit)));
        let (hit, _) = keymap.bindings_for_input(&key("cmd-w"), &[]);
        assert!(hit.iter().any(|b| b.action().partial_eq(&CloseWindow)));
    }

    #[test]
    fn the_selection_survives_a_removal_or_moves_to_the_newest_left() {
        let left = ["v0.2.49".to_string(), "v0.2.48".to_string()];
        let pick =
            |saved: Option<&str>| super::selection_after_removal(&left, saved.map(String::from));
        assert_eq!(pick(Some("v0.2.48")).as_deref(), Some("v0.2.48"));
        assert_eq!(pick(Some("v0.2.47")).as_deref(), Some("v0.2.49"));
        assert_eq!(pick(None).as_deref(), Some("v0.2.49"));
        assert_eq!(
            super::selection_after_removal(&[], Some("v0.2.47".into())),
            None
        );
    }

    #[test]
    fn the_app_menu_has_quit() {
        let menus = super::menus();
        assert_eq!(menus[0].name, "Skyline Launcher");
        assert!(matches!(
            &menus[0].items[0],
            gpui::MenuItem::Action { name, .. } if name == "Quit Skyline Launcher"
        ));
    }

    #[test]
    fn the_offline_name_is_read_from_its_key() {
        use super::player_name;
        assert_eq!(
            player_name("name:Player63010\n").as_deref(),
            Some("Player63010")
        );
        assert_eq!(player_name("name:Steve\r\n").as_deref(), Some("Steve"));
        assert_eq!(player_name("Player63010\n"), None);
        assert_eq!(player_name(""), None);
    }
}
