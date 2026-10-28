//! Content: what the selected instance plays with, all from files on this computer: its Skyline
//! mods (`horizon_launcher::mods`), resource and shader packs (`packs_page`), and each world's
//! data packs (`datapacks_page`). Every section takes files and folders from a picker or dropped
//! on it, opens its folder, turns things on or off, and removes them to the trash.
use super::*;
use gpui::ExternalPaths;
use horizon_launcher::mods::{self, Mod};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct GameMods {
    home: Option<PathBuf>,
    list: Vec<Mod>,
    fresh: bool,
    /// The mod waiting for its removal to be confirmed, by file.
    confirm_remove: Option<String>,
    /// What the last action did, or why it failed.
    note: Option<Result<String, String>>,
}

impl GameMods {
    pub(super) fn stale(&mut self) {
        self.fresh = false;
    }
}

/// Opens a picker for files or folders, then hands what was picked to `then`.
pub(super) fn pick(
    folders: bool,
    cx: &mut Context<Launcher>,
    then: impl FnOnce(&mut Launcher, Vec<PathBuf>) + 'static,
) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: !folders,
        directories: folders,
        multiple: true,
        prompt: Some("Add".into()),
    });
    cx.spawn(async move |this, cx| {
        if let Ok(Ok(Some(paths))) = paths.await {
            let _ = this.update(cx, |this, cx| {
                then(this, paths);
                cx.notify();
            });
        }
    })
    .detach();
}

impl Launcher {
    fn load_mods_list(&mut self, home: PathBuf) {
        if self.game_mods.home.as_ref() != Some(&home) {
            self.game_mods = GameMods {
                home: Some(home.clone()),
                ..GameMods::default()
            };
        }
        if !self.game_mods.fresh {
            self.game_mods.list = mods::list(&home);
            self.game_mods.fresh = true;
        }
    }

    fn mod_action(&mut self, act: impl FnOnce(&std::path::Path) -> Result<String, String>) {
        let Some(home) = self.game_mods.home.clone() else {
            return;
        };
        self.game_mods.note = Some(act(&home));
        self.game_mods.confirm_remove = None;
        self.game_mods.stale();
    }

    /// Copies dropped or picked `.hmod` files and mod folders into the instance's mods.
    fn add_mods(&mut self, files: Vec<PathBuf>) {
        self.mod_action(|home| {
            let mut added = Vec::new();
            for file in &files {
                added.push(mods::add(home, file)?.title);
            }
            Ok(trf(
                "Added {mods}: turn it on below.",
                &[("mods", &added.join(", "))],
            ))
        });
    }

    /// The Content page, for the selected instance.
    pub(super) fn content_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let name = self
            .selected_instance()
            .map_or_else(|| "Skyline".to_string(), |i| i.name.clone());
        let mut page = div().flex().flex_col().gap(px(32.)).pt(px(28.)).child(page_header(
            "Content",
            &trf(
                "Mods, packs and data packs for {instance}. Add files or folders, or drop them on a section.",
                &[("instance", &name)],
            ),
            &t,
        ));
        if self.client.is_some() {
            page = page.child(notice(
                tr("The game is running: changes here apply the next time it starts.").into(),
                &t,
            ));
        }
        page = page.children(self.mods_section(cx));
        page = page.children(self.packs_section(cx));
        page.children(self.datapacks_section(cx))
    }

    fn mods_section(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let t = self.theme;
        let home = self.selected_instance()?.home.clone();
        self.load_mods_list(home.clone());
        let dir = home.join(mods::DIR);
        let header =
            div()
                .flex()
                .items_center()
                .justify_between()
                .mb(px(4.))
                .child(section_title("Mods", "box", &t))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(button("mods-add", "Add .hmod…", Kind::Ghost, &t).on_click(
                            cx.listener(|_, _: &ClickEvent, _, cx| {
                                pick(false, cx, |this, paths| this.add_mods(paths));
                            }),
                        ))
                        .child(
                            button("mods-add-folder", "Add folder…", Kind::Ghost, &t).on_click(
                                cx.listener(|_, _: &ClickEvent, _, cx| {
                                    pick(true, cx, |this, paths| this.add_mods(paths));
                                }),
                            ),
                        )
                        .child(
                            button("mods-folder", "Open folder", Kind::Ghost, &t).on_click(
                                move |_, _, cx| {
                                    let _ = std::fs::create_dir_all(&dir);
                                    cx.open_with_system(&dir);
                                },
                            ),
                        ),
                );
        let mut section = div()
            .id("mods")
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(2.))
            .mx(px(-4.))
            .border_2()
            .border_color(gpui::transparent_black())
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(t.accent).bg(t.well))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_mods(paths.paths().to_vec());
                // Not the window's import as well.
                cx.stop_propagation();
                cx.notify();
            }))
            .child(header);
        match &self.game_mods.note {
            Some(Ok(m)) => {
                section = section.child(text(theme::CAPTION).text_color(t.muted).child(m.clone()))
            }
            Some(Err(e)) => section = section.child(notice(e.clone(), &t)),
            None => {}
        }
        let list = self.game_mods.list.clone();
        if list.is_empty() {
            section = section.child(text(theme::CAPTION).text_color(t.muted).child(tr(
                "No mods yet: drop a .hmod file or a mod folder (with horizon-mod.toml) here.",
            )));
        }
        for (i, m) in list.into_iter().enumerate() {
            section = section.child(self.mod_row(i, m, cx));
        }
        Some(section)
    }

    fn mod_row(&mut self, i: usize, m: Mod, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let id = |what: &str| SharedString::from(format!("mod-{what}-{i}"));
        let picture = div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(40.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well)
            .child(blocks::icon(
                if m.folder { "folder" } else { "box" },
                18.,
                t.muted,
            ));
        let mut actions = div().flex().items_center().gap(px(6.));
        if self.game_mods.confirm_remove.as_ref() == Some(&m.file) {
            let gone = m.clone();
            actions = actions
                .child(
                    text(theme::CAPTION)
                        .text_color(t.danger)
                        .child(tr("Move it to the trash?")),
                )
                .child(
                    button(id("keep"), "Keep", Kind::Ghost, &t).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.game_mods.confirm_remove = None;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    button(id("trash"), "Move to trash", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let gone = gone.clone();
                            this.mod_action(move |home| {
                                mods::remove(home, &gone).map(|()| {
                                    trf("{name} moved to the trash.", &[("name", &gone.title)])
                                })
                            });
                            cx.notify();
                        }),
                    ),
                );
        } else {
            let toggle = switch(id("toggle"), m.enabled, &t);
            let flip = m.clone();
            let toggle = if m.id.is_none() {
                disabled(toggle)
            } else {
                toggle.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let flip = flip.clone();
                    this.mod_action(move |home| {
                        mods::set_enabled(home, &flip, !flip.enabled).map(|()| match flip.enabled {
                            true => trf("{name} is off.", &[("name", &flip.title)]),
                            false => trf("{name} is on.", &[("name", &flip.title)]),
                        })
                    });
                    cx.notify();
                }))
            };
            let file = m.file.clone();
            let remove = button(id("remove"), "Remove", Kind::Ghost, &t).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.game_mods.confirm_remove = Some(file.clone());
                    cx.notify();
                },
            ));
            actions = actions.child(toggle).child(remove);
        }
        let facts = match (&m.problem, m.description.is_empty()) {
            (Some(p), _) => format!("{} · {p}", m.file),
            (None, true) => m.file.clone(),
            (None, false) => format!("{} · {}", m.description, m.file),
        };
        plate(&t)
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(14.))
            .py(px(8.))
            .child(picture)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        blocks::pixel(14., FontWeight::MEDIUM)
                            .text_color(if m.enabled { t.ink } else { t.muted })
                            .truncate()
                            .child(m.title.clone()),
                    )
                    .child(
                        text(theme::CAPTION)
                            .text_color(if m.problem.is_some() {
                                t.danger
                            } else {
                                t.muted
                            })
                            .truncate()
                            .child(facts),
                    ),
            )
            .child(actions)
    }

    /// Each world of the instance with its data packs: a world takes dropped packs, and its
    /// Data packs… button opens them under it.
    fn datapacks_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        let home = self.selected_instance()?.home.clone();
        self.load_worlds(home.clone(), cx);
        let list = self.worlds.list.clone();
        let mut section = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(section_title("Data packs", "book-open", &t).mb(px(4.)));
        if list.is_empty() {
            return Some(
                section.child(text(theme::CAPTION).text_color(t.muted).child(tr(
                    "Data packs belong to a world: they can be added here once a world exists.",
                ))),
            );
        }
        for (i, w) in list.into_iter().enumerate() {
            let open = self.world_packs_open(&w);
            let world = w.clone();
            let row = plate(&t)
                .flex()
                .items_center()
                .gap(px(14.))
                .px(px(14.))
                .py(px(8.))
                .child(blocks::icon("globe", 18., t.muted))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            blocks::pixel(14., FontWeight::MEDIUM)
                                .text_color(t.ink)
                                .truncate()
                                .child(w.name.clone()),
                        )
                        .child(
                            text(theme::CAPTION)
                                .text_color(t.muted)
                                .truncate()
                                .child(w.folder.clone()),
                        ),
                )
                .child(
                    button(
                        SharedString::from(format!("content-world-packs-{i}")),
                        if open { "Close" } else { "Data packs…" },
                        Kind::Ghost,
                        &t,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.toggle_world_packs(&world);
                            cx.notify();
                        },
                    )),
                );
            section = section
                .child(self.world_drop_target(i, &w, row, cx))
                .children(self.world_packs_panel(&w, cx));
        }
        Some(section)
    }
}
