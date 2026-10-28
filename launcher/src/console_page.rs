//! Home's Console: the game's output of this run as it comes, with a search, Follow (keep the
//! newest line in view), Copy and the logs folder. Nothing is uploaded anywhere.
use super::servers_page::Field;
use super::*;
use gpui::{ClipboardItem, ScrollHandle};
use horizon_launcher::game_log;

pub(super) struct Console {
    open: bool,
    follow: bool,
    scroll: ScrollHandle,
    /// Lines and search at the last draw: a change means scroll down when following.
    seen: (usize, String),
}

impl Default for Console {
    fn default() -> Self {
        Console {
            open: std::env::var("SKYLINE_LAUNCHER_CONSOLE").is_ok(),
            follow: true,
            scroll: ScrollHandle::new(),
            seen: (0, String::new()),
        }
    }
}

impl Launcher {
    /// Home's Console section, once the game has said something (or is running).
    pub(super) fn console_section(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let t = self.theme;
        if self.prepare_log.is_empty() && self.client.is_none() {
            return None;
        }
        let open = self.console.open;
        let toggle = button(
            "console-toggle",
            if open { "Hide" } else { "Show" },
            Kind::Ghost,
            &t,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.console.open = !this.console.open;
            cx.notify();
        }));
        let count = self.prepare_log.len();
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(section_title("Console", "terminal", &t))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        text(theme::CAPTION)
                            .text_color(t.muted)
                            .child(match self.client {
                                Some(_) => format!("{count} lines, the game is running"),
                                None => format!("{count} lines from the last run"),
                            }),
                    )
                    .child(toggle),
            );
        let mut section = div().flex().flex_col().gap(px(10.)).child(header);
        if !open {
            return Some(section);
        }
        let search = self.servers.value(Field::ConsoleSearch).trim().to_string();
        let lines: Vec<String> = self
            .prepare_log
            .iter()
            .filter(|l| game_log::matches(l, &search))
            .cloned()
            .collect();
        let now = (count, search);
        if self.console.follow && now != self.console.seen {
            self.console.scroll.scroll_to_bottom();
        }
        self.console.seen = now;
        let follow = div()
            .id("console-follow")
            .flex()
            .items_center()
            .gap(px(8.))
            .cursor_pointer()
            .child(switch("console-follow-switch", self.console.follow, &t))
            .child(text(theme::LABEL).text_color(t.ink_soft).child("Follow"))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.console.follow = !this.console.follow;
                this.console.seen.0 = usize::MAX;
                cx.notify();
            }));
        let copied = lines.join("\n");
        let tools = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(10.))
            .child(self.field(Field::ConsoleSearch, "Search the log", 260., cx))
            .child(follow)
            .child(div().flex_1())
            .child(
                button("console-copy", "Copy", Kind::Ghost, &t).on_click(cx.listener(
                    move |_this, _: &ClickEvent, _, cx| {
                        copy_to_clipboard(cx, ClipboardItem::new_string(copied.clone()));
                        cx.notify();
                    },
                )),
            )
            .child({
                let logs = self
                    .client_home
                    .clone()
                    .or_else(|| self.selected_instance().map(|i| i.home.clone()))
                    .unwrap_or_else(backend::game_home)
                    .join("logs");
                button("console-folder", "Open logs folder", Kind::Ghost, &t).on_click(
                    move |_, _, cx| {
                        let _ = std::fs::create_dir_all(&logs);
                        cx.open_with_system(&logs);
                    },
                )
            });
        section = section.child(tools);
        let shown = lines.len().saturating_sub(LOG_LINES);
        let body = div()
            .id("console-lines")
            .track_scroll(&self.console.scroll)
            .h(px(320.))
            .overflow_y_scroll()
            .p(px(10.))
            .border_2()
            .border_color(t.outline)
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .children(lines.into_iter().skip(shown).map(|line| {
                let color = if line.contains("/ERROR]") || line.contains("ERROR:") {
                    t.danger
                } else if line.contains("/WARN]") {
                    t.warn
                } else {
                    t.ink_soft
                };
                div().text_color(color).child(line)
            }));
        Some(section.child(body))
    }
}
