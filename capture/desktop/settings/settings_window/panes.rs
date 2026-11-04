// MARK: Panes

impl SettingsWindow {
    fn sync_meter(&mut self) {
        let wanted = (self.pane == Pane::Recording)
            .then(|| self.state.as_ref()?.microphone.meter.clone())
            .flatten();
        if self.meter.as_ref().map(|(id, _)| id) == wanted.as_ref() {
            return;
        }
        self.meter = wanted.map(|id| (id.clone(), crate::capture::engine::runtime::audio::mic::Meter::start(id)));
    }

    fn sidebar(&self, theme: Theme, width: f32, cx: &mut Context<Self>) -> Div {
        div()
            .flex_none()
            .w(px(width))
            .h_full()
            .bg(theme.sidebar())
            .flex()
            .flex_col()
            .child(drag_strip("settings-drag-sidebar"))
            .child(
                div()
                    .px(px(20.0))
                    .pt(px(8.0))
                    .pb(px(28.0))
                    .child(img(Arc::clone(&self.wordmark)).w(px((width - 40.0).min(150.0))).h(px(35.0)))
                    .child(div().pt(px(12.0)).text_size(px(10.0))
                        .text_color(rgb(0xb7a5d8)).child("CAPTURE SETTINGS")),
            )
            .child(self.item(theme, Pane::General, cx))
            .child(self.item(theme, Pane::Recording, cx))
            .child(self.item(theme, Pane::Storage, cx))
            .child(self.item(theme, Pane::Permissions, cx))
            .child(self.item(theme, Pane::Agents, cx))
            .child(self.item(theme, Pane::About, cx))
    }

    fn item(&self, theme: Theme, pane: Pane, cx: &mut Context<Self>) -> Stateful<Div> {
        let (path, _) = pane_icon(pane);
        let selected = self.pane == pane;
        div()
            .id(SharedString::from(format!("settings-pane-{}", pane_title(pane))))
            .h(px(44.0))
            .px(px(17.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .border_l_1()
            .border_color(if selected { rgb(0xb7a5d8) } else { theme.sidebar() })
            .when(selected, |d| d.bg(theme.selection()))
            .when(!selected, |d| d.hover(|s| s.bg(rgb(0x29232f))))
            .cursor_pointer()
            .child(icon(path, 16.0, rgb(0xb7a5d8)))
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(rgb(0xf5f0e8))
                    .child(pane_title(pane)),
            )
            .on_click(cx.listener(move |view, _, _, cx| {
                if view.recording().is_some() {
                    view.act(Action::CancelShortcut);
                }
                view.menu = None;
                if pane == Pane::Storage {
                    view.act(Action::Navigate { pane });
                    return;
                }
                view.pane = pane;
                view.sync_meter();
                cx.notify();
            }))
    }

    fn content(&self, theme: Theme, state: &ViewState, width: f32, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let body = match self.pane {
            Pane::General => self.general(theme, state, window, cx),
            Pane::Recording => self.recording_pane(theme, state, cx),
            Pane::Storage => self.storage(theme, state, cx),
            Pane::Permissions => self.permissions(theme, state, cx),
            Pane::Agents => self.agents(theme, state, cx),
            Pane::About => self.about(theme, state, width < 500.0, cx),
        };
        div()
            .id("settings-content")
            .w(px(width))
            .h_full()
            .bg(theme.window())
            .relative()
            .flex()
            .flex_col()
            .on_click(cx.listener(|view, _, _, _| {
                if view.recording().is_some() {
                    view.act(Action::CancelShortcut);
                }
            }))
            .child(
                div()
                    .relative()
                    .h(px(92.0))
                    .flex_none()
                    .px(px(INSET))
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(31.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text())
                            .child(pane_title(self.pane)),
                    )
                    .child(div().flex_1())
                    .children(state.connect.as_ref().filter(|connect| connect.presentation.account_menu).map(|connect| {
                        let options = connect.presentation.buttons.iter()
                            .filter(|b| connect.presentation.menu_actions.contains(&b.command))
                            .map(|b| (serde_json::to_value(b.command).unwrap().as_str().unwrap().to_owned(), b.title.clone(), b.enabled))
                            .collect::<Vec<_>>();
                        div().id("settings-connect-account").track_focus(&self.connect_focus).tab_index(0).size(px(44.0)).rounded_full()
                            .bg(theme.selection()).text_size(px(18.0)).cursor_pointer()
                            .when(self.connect_focus.is_focused(window), |d| d.border_2().border_color(theme.text()))
                            .flex().items_center().justify_center()
                            .when_some(connect.presentation.account_initial.clone(), |d, initial| d.child(initial))
                            .when(connect.presentation.account_initial.is_none(), |d| d.child(icon(crate::capture::desktop::icons::PERSON, 18.0, theme.text())))
                            .on_click(cx.listener(move |view, event, window, cx| {
                                cx.stop_propagation();
                                window.focus(&view.connect_focus);
                                view.open_menu("connect", options.clone(), None, event);
                                cx.notify();
                            }))
                    }))
                    .child(drag_strip("settings-drag-title").absolute().top_0().left_0()),
            )
            .child(div().w_full().h(px(1.0)).bg(theme.group_line()))
            .child(
                div()
                    .id("settings-pane-scroll")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(INSET))
                    .pt(px(pane_top_inset(self.pane)))
                    .flex()
                    .flex_col()
                    .child(body),
            )
    }

    fn connect_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if let Some(menu) = self.menu.as_mut().filter(|m| m.key == "connect") {
            match key {
                "escape" => { self.menu = None; window.focus(&self.connect_focus); }
                "down" | "up" => {
                    let enabled = menu.options.iter().filter(|o| o.2).map(|o| o.0.clone()).collect::<Vec<_>>();
                    if !enabled.is_empty() {
                        let index = menu.selected.as_ref().and_then(|id| enabled.iter().position(|x| x == id));
                        let next = match (key, index) {
                            ("down", Some(i)) => (i + 1) % enabled.len(),
                            ("up", Some(i)) => (i + enabled.len() - 1) % enabled.len(),
                            ("up", None) => enabled.len() - 1,
                            _ => 0,
                        };
                        menu.selected = Some(enabled[next].clone());
                    }
                }
                "enter" | "space" => {
                    let command = menu.selected.as_ref().or_else(|| menu.options.iter().find(|o| o.2).map(|o| &o.0))
                        .and_then(|id| serde_json::from_value(serde_json::Value::String(id.clone())).ok());
                    self.menu = None;
                    if let Some(command) = command { self.act(Action::Connect { command }); }
                    window.focus(&self.connect_focus);
                }
                _ => return false,
            }
            cx.notify(); return true;
        }
        false
    }

    fn connect_card(&self, connect: &super::connect::View, window: &Window, cx: &mut Context<Self>) -> Div {
        use super::connect::{Command, Tone};
        let p = &connect.presentation;
        let mut content = div().flex_1().min_w_0().flex().flex_col().gap(px(16.0));
        let mut headline = div().flex().items_start().gap(px(8.0));
        if p.tone != Tone::Neutral {
            headline = headline.child(div().size(px(24.0)).flex_none()
                .text_size(px(20.0)).text_color(rgb(if p.tone == Tone::Warning { 0x020102 } else { p.tone.color() }))
                .when(p.tone == Tone::Warning, |d| d.bg(rgb(0xffbe2e)))
                .flex().items_center().justify_center().child(p.tone.symbol()));
        }
        content = content.child(headline.child(div().flex_1()
            .text_size(px(if p.compact { 18.0 } else { 36.0 }))
            .line_height(px(if p.compact { 27.0 } else { 40.0 }))
            .font_weight(FontWeight::BOLD).child(p.headline.clone())))
            .child(div().text_size(px(18.0)).line_height(px(27.0)).child(p.detail.clone()));
        if !p.note.is_empty() {
            content = content.child(div().text_size(px(18.0)).line_height(px(27.0)).child(p.note.clone()));
        }
        let mut actions = div().flex().flex_wrap().gap(px(16.0));
        for (index, item) in p.buttons.iter().filter(|b| !p.menu_actions.contains(&b.command)).enumerate() {
            let command = item.command;
            let secondary = command == Command::Cancel;
            let destructive = command == Command::Disconnect;
            actions = actions.child(div().id(("connect-action", index)).track_focus(&self.connect_action_focus[index].clone().tab_stop(item.enabled))
                .h(px(44.0)).px(px(16.0)).flex_none().flex().items_center()
                .bg(rgb(if secondary || !item.enabled { 0xf5f0e8 } else if destructive { 0xb42318 } else { 0x7a58c7 }))
                .border_1().border_color(rgb(if secondary || !item.enabled { 0xcbc4bc } else if destructive { 0xb42318 } else { 0x7a58c7 }))
                .text_size(px(18.0)).text_color(if !item.enabled { rgba(0x02010273) } else { rgb(if secondary { 0x020102 } else { 0xf5f0e8 }) })
                .when(self.connect_action_focus[index].is_focused(window), |d| d.border_2().border_color(rgb(0x020102)))
                .child(item.title.clone())
                .when(item.enabled, |d| d.tab_index(0).cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, _| {
                        window.focus(&view.connect_action_focus[index]); view.act(Action::Connect { command });
                    }))));
        }
        content = content.child(actions);
        div().w_full().mb(px(24.0)).p(px(24.0))
            .rounded(px(8.0)).bg(rgb(0xf5f0e8)).text_color(rgb(0x020102))
            .border_1().border_color(rgb(0xcbc4bc))
            .when(p.tone != Tone::Neutral, |d| d.border_l_4().border_color(rgb(p.tone.color())))
            .flex().items_center().gap(px(24.0)).child(content)
            .when(p.artwork, |d| {
                static FROGS: std::sync::OnceLock<Arc<Image>> = std::sync::OnceLock::new();
                let image = FROGS.get_or_init(|| Arc::new(Image::from_bytes(ImageFormat::Png,
                    include_bytes!("../../../../shared/assets/brand/connect-frogs.png").to_vec()))).clone();
                let width = if p.compact { 100.0 } else { 144.0 };
                d.child(img(image).w(px(width)).h(px(width * 406.0 / 758.0)).flex_none().object_fit(gpui::ObjectFit::Contain))
            })
    }

    fn general(&self, theme: Theme, state: &ViewState, window: &Window, cx: &mut Context<Self>) -> Div {
        let login_on = state.login.on;
        let mut column = div().flex().flex_col();
        if let Some(connect) = &state.connect {
            column = column.child(self.connect_card(connect, window, cx));
        }
        column = column.child(group(
            theme,
            vec![
                row(
                    theme,
                    "Launch at login",
                    state.login.available,
                    switch(theme, "settings-login", login_on, state.login.available).when(
                        state.login.available,
                        |d| {
                            d.on_click(cx.listener(move |view, _, _, _| {
                                view.set("launchAtLogin", (!login_on).into());
                            }))
                        },
                    ),
                )
                .into_any_element(),
            ],
        ));
        if state.login.needs_approval {
            column = column.child(caption(
                theme,
                if WINDOWS {
                    "Turn Apparent on in Settings › Apps › Startup."
                } else if LINUX {
                    "Your desktop's startup settings turned Apparent off. Turn it on there."
                } else {
                    "Allow Apparent in System Settings › General › Login Items."
                },
                false,
            ));
        } else if !state.login.available {
            column = column.child(caption(
                theme,
                "Launch at login works in the installed Apparent.app.",
                false,
            ));
        }
        for shortcut in &state.shortcuts {
            column = column.child(self.recorder(theme, shortcut, cx));
        }
        let error_reporting = state.error_reporting;
        column = column
            .child(div().mt(px(14.0)).child(group(theme, vec![
                row(
                    theme,
                    "Send technical error reports",
                    true,
                    switch(theme, "settings-error-reporting", error_reporting, true)
                        .on_click(cx.listener(move |view, _, _, _| {
                            view.set("errorReporting", (!error_reporting).into());
                        })),
                ).into_any_element(),
            ])))
            .child(caption(theme, "No recordings, screenshots, transcripts or clipboard contents are sent.", false));
        if let Some(account) = state.hosted.as_ref().filter(|_| state.connect.is_none()) {
            let available = account.server.is_some();
            if state.connect.is_none() {
            column = column.child(div().mt(px(20.0)).child("Hosted account"))
                .child(caption(theme, if account.connected { "Connected. Captures stay private until you click Copy link." } else {
                    "Connect to upload screen, voice, transcript and bundle media. New captures upload privately; only the selected capture is shared."
                }, false))
                .child(button(theme, "hosted-connect", "Connect and allow uploads", available && !account.connected)
                    .when(available && !account.connected, |button| button.on_click(cx.listener(|view, _, _, _| view.act(Action::ConnectHosted)))));
            }
            column = column.child(button(theme, "hosted-shares", "Manage shared links", available)
                    .when(available, |button| button.on_click(cx.listener(|view, _, _, _| view.act(Action::ManageSharedLinks)))))
                .child(caption(theme, "Anyone with a link can view and download that capture. Stopping sharing cannot recall downloads or cached previews.", false));
            if !available && state.connect.is_none() {
                column = column.child(caption(theme, "Set the API server first with apparent cloud login --server <API origin> --scope read.", false));
            }
        }
        if !WINDOWS {
            let on = state.lower_playback;
            let available = state.playback_available;
            let toggle_enabled = available || on;
            let level_enabled = on && available;
            let level = state.playback_level.to_string();
            let options = [0, 10, 20, 35, 50, 75, 100]
                .into_iter()
                .map(|n| (n.to_string(), format!("{n}%"), true))
                .collect::<Vec<_>>();
            column = column
                .child(header(theme, "Other audio"))
                .child(group(theme, vec![
                    row(
                        theme,
                        "Lower other audio while recording",
                        toggle_enabled,
                        switch(theme, "settings-lower-playback", on, toggle_enabled)
                            .when(toggle_enabled, |d| d.on_click(cx.listener(move |view, _, _, _| {
                                view.set("lowerPlayback", (!on).into());
                            }))),
                    ).into_any_element(),
                    row(
                        theme,
                        "Lower to",
                        level_enabled,
                        popup(theme, "settings-playback-level", &format!("{}%", state.playback_level))
                            .when(!level_enabled, |d| d.opacity(0.4))
                            .when(level_enabled, |d| d.on_click(cx.listener(move |view, event: &ClickEvent, _, cx| {
                                view.open_menu("playbackLevel", options.clone(), Some(level.clone()), event);
                                cx.notify();
                            }))),
                    ).into_any_element(),
                ]))
                .child(caption(theme, &state.playback_note, false));
        }
        column
    }

    fn recorder(&self, theme: Theme, shortcut: &Shortcut, cx: &mut Context<Self>) -> Div {
        let kind = shortcut.kind;
        let recording = shortcut.recording;
        let live = self.live;
        let lit = |on: bool, held: bool| if recording { held } else { on };
        let reset_hidden = shortcut.is_default || recording || shortcut.externally_managed;
        let off = shortcut.label.is_empty();
        let clear_shown = shortcut.clearable && !off && !recording;
        let caption_text = if recording && shortcut.clearable {
            format!(
                "Press the new shortcut. Esc cancels, {} turns it off.",
                if WINDOWS || LINUX { "Backspace" } else { "Delete" }
            )
        } else if recording {
            "Press the new shortcut. Esc cancels.".to_owned()
        } else if let Some(error) = &shortcut.error {
            error.clone()
        } else if shortcut.externally_managed {
            "Set in Hyprland bindings.lua. Edit that binding, then restart Apparent.".to_owned()
        } else if kind == ShortcutKind::Paste {
            "Pastes the last capture into the app in front.".to_owned()
        } else {
            "Records the screen. Press again to finish.".to_owned()
        };
        let key = if recording {
            "…".to_owned()
        } else {
            shortcut.key.clone()
        };
        // macOS draws ⇧ ⌃ ⌥ ⌘; Windows and Linux name their keys, in the order they sit on the
        // keyboard.
        let caps = if off && !recording {
            div()
                .h(px(52.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(14.0))
                .text_color(theme.secondary())
                .child("Set shortcut")
        } else if shortcut.externally_managed {
            // A system-owned chord is a label, not a five-key recorder. Show only
            // the keys that are actually held, so the key stays visible in a
            // narrow tiled Settings window.
            div()
                .flex()
                .gap(px(6.0))
                .when(shortcut.shift, |d| {
                    d.child(keycap(theme, "Shift", None, true, false))
                })
                .when(shortcut.control, |d| {
                    d.child(keycap(theme, "Ctrl", None, true, false))
                })
                .when(shortcut.command, |d| {
                    d.child(keycap(theme, "Super", None, true, false))
                })
                .when(shortcut.option, |d| {
                    d.child(keycap(theme, "Alt", None, true, false))
                })
                .child(keycap(theme, &key, None, true, true))
        } else if WINDOWS || LINUX {
            let super_key = if LINUX { "Super" } else { "Win" };
            div()
                .flex()
                .gap(px(6.0))
                .child(keycap(theme, "Shift", None, lit(shortcut.shift, live.shift), false))
                .child(keycap(theme, "Ctrl", None, lit(shortcut.control, live.control), false))
                .child(keycap(theme, super_key, None, lit(shortcut.command, live.platform), false))
                .child(keycap(theme, "Alt", None, lit(shortcut.option, live.alt), false))
                .child(keycap(theme, &key, None, !recording, true))
        } else {
            div()
                .flex()
                .gap(px(6.0))
                .child(keycap(theme, "⇧", None, lit(shortcut.shift, live.shift), false))
                .child(keycap(theme, "⌃", Some("ctrl"), lit(shortcut.control, live.control), false))
                .child(keycap(theme, "⌥", Some("opt"), lit(shortcut.option, live.alt), false))
                .child(keycap(theme, "⌘", Some("cmd"), lit(shortcut.command, live.platform), false))
                .child(keycap(theme, &key, None, !recording, true))
        };
        let id = |name: &str| SharedString::from(format!("settings-{name}-{kind:?}"));
        div()
            .flex()
            .flex_col()
            .child(
                header(theme, &shortcut.title).child(
                    div()
                        .flex()
                        .gap(px(6.0))
                        .child(
                            div()
                                .id(id("reset"))
                                .size(px(20.0))
                                .rounded_full()
                                .bg(theme.control())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(crate::capture::desktop::icons::RESET, 10.0, theme.secondary()))
                                .when(reset_hidden, |d| d.opacity(0.0))
                                .when(!reset_hidden, |d| {
                                    d.cursor_pointer().on_click(cx.listener(
                                        move |view, _, _, _| {
                                            view.act(Action::ResetShortcut { kind });
                                        },
                                    ))
                                }),
                        )
                        .when(clear_shown, |d| {
                            d.child(
                                div()
                                    .id(id("clear"))
                                    .size(px(20.0))
                                    .rounded_full()
                                    .bg(theme.control())
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .child(icon(crate::capture::desktop::icons::X, 10.0, theme.secondary()))
                                    .on_click(cx.listener(move |view, _, _, _| {
                                        view.act(Action::ClearShortcut { kind });
                                    })),
                            )
                        }),
                ),
            )
            .child(
                div()
                    .id(id("recorder"))
                    .p(px(7.0))
                    .rounded(px(RADIUS + 2.0))
                    .bg(theme.group())
                    .border_1()
                    .border_color(if recording {
                        theme.accent()
                    } else {
                        theme.group_line()
                    })
                    .when(!shortcut.externally_managed, |d| d.cursor_pointer())
                    .child(caps)
                    .when(!shortcut.externally_managed, |d| {
                        d.on_click(cx.listener(move |view, _, window, cx| {
                            cx.stop_propagation();
                            view.menu = None;
                            if recording {
                                view.act(Action::CancelShortcut);
                            } else {
                                window.focus(&view.focus);
                                view.act(Action::RecordShortcut { kind });
                            }
                        }))
                    }),
            )
            .child(
                div()
                    .h(px(26.0))
                    .child(caption(theme, &caption_text, shortcut.error.is_some() && !recording)),
            )
    }

    fn recording_pane(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let mic = &state.microphone;
        let inputs: Vec<(String, String, bool)> = mic
            .options
            .iter()
            .map(|option| (option.id.clone(), option.title.clone(), option.enabled))
            .collect();
        let selected = mic.selected.clone();
        let level = self.meter.as_ref().map_or(0.0, |(_, meter)| meter.level());
        let t = &state.transcriber;
        let hex = t.engine == "hex";
        let command = t.engine == "command";
        let local = crate::capture::engine::runtime::audio::transcription::transcribe::LOCAL_ENGINE_TITLE;
        let mut engines = vec![
            ("apple".to_owned(), local.to_owned(), true),
            ("hex".to_owned(), t.hex_title.clone(), t.hex_enabled),
        ];
        if let Some(program) = &t.command {
            engines.push(("command".to_owned(), format!("Command ({program})"), true));
        }
        let engine_title = match (&t.command, hex) {
            (Some(program), _) if command => format!("Command ({program})"),
            (_, true) => "Hex".to_owned(),
            _ => local.to_owned(),
        };
        let chosen = t.models.iter().find(|m| m.chosen);
        let mut rows = vec![row(
            theme,
            "Transcription",
            true,
            popup(theme, "settings-transcriber", &engine_title).on_click(
                cx.listener({
                    let selected = t.engine.clone();
                    move |view, event: &ClickEvent, _, cx| {
                        view.open_menu("transcriber", engines.clone(), Some(selected.clone()), event);
                        cx.notify();
                    }
                }),
            ),
        )
        .into_any_element()];
        if hex && !t.models.is_empty() {
            let options: Vec<(String, String, bool)> = t
                .models
                .iter()
                .map(|m| (m.id.clone(), m.title.clone(), true))
                .collect();
            let selected = chosen.map(|m| m.id.clone());
            rows.push(
                row(
                    theme,
                    "Hex model",
                    true,
                    popup(
                        theme,
                        "settings-hex-model",
                        chosen.map_or("Choose", |m| m.title.as_str()),
                    )
                    .on_click(cx.listener(
                        move |view, event: &ClickEvent, _, cx| {
                            view.open_menu("hexModel", options.clone(), selected.clone(), event);
                            cx.notify();
                        },
                    )),
                )
                .into_any_element(),
            );
        }
        let here = if LINUX { "this computer" } else { "this PC" };
        let note = match (WINDOWS || LINUX, t.hex_enabled || hex) {
            _ if command => {
                let fallback = if WINDOWS || LINUX { "Parakeet" } else { "Apple Speech" };
                format!(
                "Command runs transcribeCommand from config.json. If it fails, that recording uses {fallback}."
                )
            }
            (true, true) => format!(
                "Parakeet runs on {here} and times each word. Hex uses a model you have in Hex, and its word times are estimates."
            ),
            (true, false) => format!(
                "Parakeet runs on {here} and times each word. Apparent downloads its 730 MB model once, unless Hex already has it. Hex shows up here while Hex is running."
            ),
            (false, true) => {
                "Apple Speech times each word. Hex uses a model you have in Hex, and its word times are estimates.".to_owned()
            }
            (false, false) => {
                "Apple Speech runs on this Mac and times each word. Hex shows up here while Hex is running.".to_owned()
            }
        };
        let speech_status = t.speech.clone().unwrap_or_default();
        let speech = match speech_status.state.as_str() {
            _ if !(WINDOWS || LINUX) || hex || command => None,
            "downloading" => Some((
                format!(
                    "Downloading the speech model: {:.0}%",
                    (speech_status.progress * 100.0).clamp(0.0, 100.0)
                ),
                false,
            )),
            "failed" => Some((
                format!(
                    "The speech model couldn't be downloaded{}. Apparent tries again in a few minutes.",
                    speech_status
                        .error
                        .as_deref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                ),
                true,
            )),
            _ => None,
        };
        let video = state.video_export;
        #[cfg(target_os = "linux")]
        let decoder_warning = crate::capture::desktop::linux::media::decoder_warning();
        #[cfg(target_os = "linux")]
        let encoder_warning = crate::capture::desktop::linux::media::encoder_warning();
        #[cfg(not(target_os = "linux"))]
        let encoder_warning: Option<String> = None;
        #[cfg(not(target_os = "linux"))]
        let decoder_warning: Option<String> = None;
        div()
            .flex()
            .flex_col()
            .child(header(theme, "Microphone"))
            .child(group(theme, vec![row(
                theme, "Input", true,
                popup(theme, "settings-microphone", &mic.value).on_click(cx.listener(
                    move |view, event: &ClickEvent, _, cx| {
                        view.open_menu("microphone", inputs.clone(), Some(selected.clone()), event);
                        cx.notify();
                    },
                )),
            ).into_any_element()]))
            .when_some(mic.note.clone(), |pane, note| pane.child(caption(theme, &note, true)))
            .when(mic.meter.is_some(), |pane| pane.child(
                div().mx(px(8.0)).mt(px(8.0)).w_full().max_w(px(260.0)).h(px(7.0))
                    .rounded_full().bg(theme.track()).child(
                        div().w(relative(level.clamp(0.0, 1.0))).h(px(7.0))
                            .rounded_full().bg(theme.accent())
                    ),
            ))
            .child(caption(theme, "If the chosen input cannot open, recording uses the system default. The level shows input, not speech clarity.", false))
            .child(header(theme, "Transcription"))
            .child(group(theme, rows))
            .child(caption(theme, &note, false))
            .when_some(encoder_warning, |pane, warning| {
                pane.child(caption(theme, &warning, true))
            })
            .when_some(decoder_warning, |pane, warning| {
                pane.child(caption(theme, &warning, true))
            })
            .when_some(speech, |pane, (text, error)| {
                pane.child(caption(theme, &text, error))
            })
            .child(header(theme, "Experimental"))
            .child(group(
                theme,
                vec![
                    row(
                        theme,
                        "Video export",
                        true,
                        switch(theme, "settings-video-export", video, true).on_click(
                            cx.listener(move |view, _, _, _| {
                                view.set("videoExport", (!video).into());
                            }),
                        ),
                    )
                    .into_any_element(),
                ],
            ))
            .child(caption(
                theme,
                "Adds Export Last Capture as Video… to the menu.",
                false,
            ))
    }

    fn storage(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let r = &state.retention;
        let current = r
            .options
            .iter()
            .find(|o| Some(&o.id) == r.preset.as_ref())
            .map_or("Custom", |o| o.label.as_str());
        let options: Vec<(String, String, bool)> = r
            .options
            .iter()
            .map(|o| (o.id.clone(), o.label.clone(), true))
            .collect();
        let selected = r.preset.clone();
        let show = if WINDOWS {
            "Show in File Explorer"
        } else if LINUX {
            "Show in Files"
        } else {
            "Show in Finder"
        };
        div()
            .flex()
            .flex_col()
            .child(group(
                theme,
                vec![
                    row(
                        theme,
                        "Keep recordings",
                        true,
                        popup(theme, "settings-retention", current).on_click(cx.listener(
                            move |view, event: &ClickEvent, _, cx| {
                                view.open_menu("retention", options.clone(), selected.clone(), event);
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element(),
                ],
            ))
            .when(!r.note.is_empty(), |d| d.child(caption(theme, &r.note, false)))
            .child(header(theme, "Captures"))
            .child(group(
                theme,
                vec![
                    row(
                        theme,
                        "Captures folder",
                        true,
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .text_color(theme.secondary())
                                    .child(
                                        state
                                            .captures_size
                                            .clone()
                                            .unwrap_or_else(|| "Measuring…".to_owned()),
                                    ),
                            )
                            .child(button(theme, "settings-show-captures", show, true).on_click(
                                cx.listener(|view, _, _, _| view.act(Action::ShowCaptures)),
                            )),
                    )
                    .into_any_element(),
                ],
            ))
    }

    fn permissions(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let paste = state
            .shortcuts
            .iter()
            .find(|s| s.kind == ShortcutKind::Paste)
            .map_or_else(crate::capture::desktop::settings::paste_label, |s| s.label.clone());
        let required: Vec<&PermissionRow> = state.permissions.iter().filter(|p| p.required).collect();
        let optional: Vec<&PermissionRow> =
            state.permissions.iter().filter(|p| !p.required).collect();
        let mut column = div().flex().flex_col();
        if !required.is_empty() {
            let required_rows = self.permission_lines(theme, &required, &paste, cx);
            column = column.child(group(theme, required_rows)).child(caption(
                theme,
                if WINDOWS {
                    "Apparent needs the microphone to hear you. Recording the screen, clicks and shortcuts need no permission on Windows."
                } else if LINUX && required.len() > 1 {
                    "Apparent needs the microphone to hear you, and on Wayland the input devices to see ⇧ ink, clicks and Esc over other apps."
                } else if LINUX {
                    "Apparent needs the microphone to hear you. On X11, the screen, clicks and shortcuts need no permission."
                } else {
                    "Apparent needs these three to record."
                },
                false,
            ));
        }
        if !optional.is_empty() {
            let optional_rows = self.permission_lines(theme, &optional, &paste, cx);
            column = column
                .child(header(theme, "When you use them"))
                .child(group(theme, optional_rows));
        }
        column
    }

    fn permission_lines(
        &self,
        theme: Theme,
        rows: &[&PermissionRow],
        paste: &str,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        rows.iter()
            .map(|row| self.permission_line(theme, row, paste, cx).into_any_element())
            .collect()
    }

    fn permission_line(
        &self,
        theme: Theme,
        row: &PermissionRow,
        paste: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let permission = row.permission;
        let (name, why, path, tint) = match permission {
            Permission::ScreenRecording => (
                "Screen Recording",
                "To see your screen".to_owned(),
                crate::capture::desktop::icons::SCREEN,
                (0x9e80ff, 0x7052eb),
            ),
            Permission::Microphone => (
                "Microphone",
                "To hear you".to_owned(),
                crate::capture::desktop::icons::MIC,
                ORANGE,
            ),
            Permission::InputMonitoring => (
                "Input Monitoring",
                "To see clicks and shortcuts".to_owned(),
                crate::capture::desktop::icons::CLICK,
                GREY,
            ),
            Permission::Accessibility if LINUX => (
                "Input devices",
                "For Shift ink and clicks. Run the copied command in a terminal, then log out and back in.".to_owned(),
                crate::capture::desktop::icons::CLICK,
                BLUE,
            ),
            Permission::Accessibility => (
                "Accessibility",
                format!(
                    "To press {} when you use {paste}",
                    crate::capture::engine::runtime::system_paste_label()
                ),
                crate::capture::desktop::icons::PERSON,
                BLUE,
            ),
            Permission::Camera => (
                "Camera",
                "To show a cabled iPhone".to_owned(),
                crate::capture::desktop::icons::IPHONE,
                GREEN,
            ),
            Permission::SpeechRecognition => (
                "Speech Recognition",
                "To write down what you say".to_owned(),
                crate::capture::desktop::icons::MIC,
                ORANGE,
            ),
        };
        let granted = row.status == Status::Granted;
        let id = SharedString::from(format!("settings-grant-{permission:?}"));
        let trailing: AnyElement = if granted && row.relaunch {
            button(theme, id, "Reopen Apparent", true)
                .on_click(cx.listener(|view, _, _, _| view.act(Action::Reopen)))
                .into_any_element()
        } else if granted {
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.secondary())
                .child(icon(crate::capture::desktop::icons::CHECK_CIRCLE, 14.0, theme.positive()))
                .child("Allowed")
                .into_any_element()
        } else {
            let title = if LINUX && permission == Permission::Accessibility {
                "Copy command"
            } else if row.requested || row.status == Status::Denied {
                "Open Settings"
            } else {
                "Allow"
            };
            button(theme, id, title, true)
                .on_click(cx.listener(move |view, _, _, _| {
                    #[cfg(target_os = "linux")]
                    if permission == Permission::Accessibility {
                        crate::capture::desktop::linux::privacy::copy_input_command();
                        return;
                    }
                    view.act(Action::Grant { permission });
                }))
                .into_any_element()
        };
        div()
            .min_h(px(52.0))
            .py(px(8.0))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(icon_tile(path, tint, 26.0))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .child(div().text_size(px(14.0)).text_color(theme.text()).child(name))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(theme.secondary())
                            .child(why),
                    ),
            )
            .child(trailing)
    }

    /// The agent skill's install command, with Copy. `apparent.exe` is a console program, so the
    /// CLI needs no install step on Windows (macOS links it onto PATH here).
    fn agents(&self, theme: Theme, state: &ViewState, cx: &mut Context<Self>) -> Div {
        let command = state.skill_command.clone();
        let demo_video = state.agent_demo_video;
        let line = div()
            .h(px(ROW))
            .px(px(12.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.0))
                    .font_family("Consolas")
                    .text_color(theme.text())
                    .child(command.clone()),
            )
            .child(
                button(
                    theme,
                    "settings-copy-skill",
                    if self.copied { "Copied" } else { "Copy" },
                    true,
                )
                .on_click(cx.listener(move |view, _, _, cx| {
                    if crate::shared::handoff::copy_string_to_clipboard(&command).is_ok() {
                        view.copied = true;
                        cx.notify();
                        cx.spawn(async move |view, cx| {
                            cx.background_executor()
                                .timer(std::time::Duration::from_millis(1500))
                                .await;
                            let _ = view.update(cx, |view, cx| {
                                view.copied = false;
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                })),
            )
            .into_any_element();
        div()
            .flex()
            .flex_col()
            .child(group(theme, vec![row(
                theme,
                "Ask for an Apparent video on complex changes",
                true,
                switch(theme, "settings-agent-demo-video", demo_video, true).on_click(
                    cx.listener(move |view, _, _, _| {
                        view.set("agentDemoVideo", (!demo_video).into());
                    }),
                ),
            ).into_any_element()]))
            .child(caption(theme, "Adds a video request to copied and pasted captures when the agent can record the result. On by default.", false))
            .child(header(theme, "Agent skill"))
            .child(group(theme, vec![line]))
            .child(caption(
                theme,
                "Run it in a terminal. It teaches Claude Code, OpenCode, Codex and other agents to read your captures and use apparent.",
                false,
            ))
    }

    fn about(&self, theme: Theme, state: &ViewState, compact: bool, cx: &mut Context<Self>) -> Div {
        let link = |id: &'static str, title: &str, shown: &str, url: &'static str| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(|s| s.bg(theme.ink(0.03)))
                .child(if compact {
                    div().min_h(px(62.0)).px(px(12.0)).py(px(8.0))
                        .flex().flex_col().justify_center()
                        .child(div().text_size(px(14.0)).text_color(theme.text()).child(title.to_owned()))
                        .child(div().text_size(px(12.0)).text_color(theme.secondary()).child(shown.to_owned()))
                } else {
                    row(
                    theme,
                    title,
                    true,
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(
                            div()
                                .text_size(px(14.0))
                                .text_color(theme.secondary())
                                .child(shown.to_owned()),
                        )
                        .child(icon(crate::capture::desktop::icons::ARROW_UP_RIGHT, 10.0, theme.tertiary())),
                    )
                })
                .on_click(move |_, _, _| {
                    let _ = crate::shared::handoff::open_path(std::path::Path::new(url));
                })
        };
        let mut column = div().flex().flex_col().child(group(
            theme,
            vec![
                div()
                    .min_h(px(if compact { 100.0 } else { 64.0 }))
                    .px(px(12.0))
                    .flex()
                    .when(compact, |d| d.flex_wrap())
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        img(Arc::clone(&self.icon)).size(px(40.0)).flex_none(),
                    )
                    .child(
                        div()
                            .flex_1().min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.text())
                                    .child("Apparent"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(theme.secondary())
                                    .child(format!("Version {}", state.version)),
                            ),
                    )
                    .child(
                        button(theme, "settings-updates", if state.update_check.checking { "Checking…" } else { "Check for Updates…" }, state.updates && !state.update_check.checking)
                            .when(state.updates && !state.update_check.checking, |d| {
                                d.on_click(
                                    cx.listener(|view, _, _, _| view.act(Action::CheckForUpdates)),
                                )
                            }),
                    )
                    .into_any_element(),
            ],
        ));
        if !state.updates {
            column = column.child(caption(
                theme,
                if LINUX { "Update checks work when Apparent runs from an AppImage." } else { "Updates work in the installed Apparent.app." },
                false,
            ));
        }
        if LINUX && state.main_updates {
            let chosen = state.update_channel.clone();
            column = column
                .child(header(theme, "Updates"))
                .child(group(theme, vec![row(
                    theme,
                    "Channel",
                    true,
                    popup(theme, "settings-update-channel", if chosen == "main" { "Latest main" } else { "Stable releases" })
                        .on_click(cx.listener(move |view, event: &ClickEvent, _, cx| {
                            view.open_menu(
                                "updateChannel",
                                vec![("stable".into(), "Stable releases".into(), true), ("main".into(), "Latest main".into(), true)],
                                Some(chosen.clone()),
                                event,
                            );
                            cx.notify();
                        })),
                )
                .into_any_element()]))
                .child(caption(theme, "Stable follows reviewed Linux releases. Main follows published nightly previews. Updates install on the next launch.", false));
        }
        column = column.child(div().h(px(14.0))).child(group(theme, vec![div()
            .py(px(11.0))
            .flex()
            .flex_col()
            .child(div()
                .px(px(12.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.secondary())
                .child("Enjoying Apparent?"))
            .child(row(
                theme,
                "Support the project",
                true,
                button(theme, "settings-star-github", "Star on GitHub", true)
                    .on_click(|_, _, _| {
                        let _ = crate::shared::handoff::open_path(std::path::Path::new("https://github.com/tryapparent/apparent"));
                    }),
            ))
            .child(div()
                .px(px(12.0))
                .text_size(px(12.0))
                .line_height(px(16.0))
                .text_color(theme.secondary())
                .child("A GitHub star won’t fix your bugs, but it will make our day."))
            .into_any_element()]));
        column
            .child(header(theme, "Links"))
            .child(group(
                theme,
                vec![
                    link(
                        "settings-website",
                        "Website",
                        "app.tryapparent.com",
                        "https://app.tryapparent.com",
                    )
                    .into_any_element(),
                    link(
                        "settings-source",
                        "Source",
                        if compact { "tryapparent/apparent" } else { "github.com/tryapparent/apparent" },
                        "https://github.com/tryapparent/apparent",
                    )
                    .into_any_element(),
                    row(
                        theme,
                        "Welcome tour",
                        true,
                        button(theme, "settings-tour", "Show", true)
                            .on_click(cx.listener(|view, _, _, _| view.act(Action::Tour))),
                    )
                    .into_any_element(),
                ],
            ))
    }

    fn update_result_layer(&self, theme: Theme, state: &ViewState, window: &Window, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let result = state.update_check.result?;
        // GPUI's image sampler is linear. Keep the extracted concept scenes at
        // native device pixels even at fractional desktop scales.
        let frog_width = px(256.0 / window.scale_factor());
        let frog_height = px(192.0 / window.scale_factor());
        Some(div()
            .id("settings-update-result-layer")
            .absolute().top_0().left_0().size_full()
            .occlude().bg(theme.ink(0.45))
            .flex().items_center().justify_center()
            .child(div()
                .id("settings-update-result")
                .w(px(340.0)).p(px(24.0))
                .bg(theme.window())
                .border_1().border_color(theme.text())
                .flex().flex_col().gap(px(16.0))
                .child(div().flex().justify_center().child(img(update_frog(result)).w(frog_width).h(frog_height).flex_shrink_0()))
                .child(div().text_center().text_size(px(18.0)).font_weight(FontWeight::MEDIUM)
                    .text_color(if result == UpdateResult::Failed { theme.negative() } else { theme.text() })
                    .child(result.label()))
                .child(div().flex().justify_end().gap(px(8.0))
                    .when(result == UpdateResult::Available, |row| row
                        .child(update_choice_button(theme, "settings-update-later", "Update later", false, &self.update_later_focus, self.update_later_focus.is_focused(window))
                            .on_click(cx.listener(|view, _, _, _| view.act(Action::UpdateLater))))
                        .child(update_choice_button(theme, "settings-update-now", "Update now", true, &self.update_now_focus, self.update_now_focus.is_focused(window))
                            .on_click(cx.listener(|view, _, _, _| view.act(Action::UpdateNow)))))
                    .when(result != UpdateResult::Available, |row| row
                        .child(update_choice_button(theme, "settings-update-dismiss", "OK", false, &self.update_dismiss_focus, self.update_dismiss_focus.is_focused(window))
                            .on_click(cx.listener(|view, _, _, _| view.act(Action::DismissUpdateCheck))))))
        ))
    }

    fn open_menu(
        &mut self,
        key: &'static str,
        options: Vec<(String, String, bool)>,
        selected: Option<String>,
        event: &ClickEvent,
    ) {
        self.menu = Some(Menu {
            key,
            options,
            selected,
            at: event.position(),
        });
    }

    /// The open popup's choices, under the pointer, and a catcher for clicks outside it.
    fn menu_layer(
        &self,
        theme: Theme,
        viewport: gpui::Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let menu = self.menu.as_ref()?;
        let item_h = if menu.key == "connect" { 44.0 } else { 26.0 };
        let w = if menu.key == "connect" { 300.0 } else { 220.0 };
        let h = menu.options.len() as f32 * item_h + 10.0;
        let x = f32::from(menu.at.x).min(f32::from(viewport.width) - w - 8.0).max(8.0);
        let y = (f32::from(menu.at.y) - item_h * 0.5)
            .min(f32::from(viewport.height) - h - 8.0)
            .max(8.0);
        let key = menu.key;
        let mut list = div()
            .id("settings-menu")
            .occlude()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .py(px(5.0))
            .rounded(px(8.0))
            .bg(theme.menu())
            .border_1()
            .border_color(theme.control_line())
            .shadow(vec![BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 0.3),
                offset: point(px(0.0), px(6.0)),
                blur_radius: px(18.0),
                spread_radius: px(0.0),
            }])
            .flex()
            .flex_col();
        for (index, (id, title, enabled)) in menu.options.iter().enumerate() {
            let chosen = menu.selected.as_ref() == Some(id);
            let id = id.clone();
            let enabled = *enabled;
            list = list.child(
                div()
                    .id(("settings-menu-item", index))
                    .h(px(item_h))
                    .mx(px(5.0))
                    .px(px(8.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(if key == "connect" { 18.0 } else { 13.0 }))
                    .text_color(if key == "connect" && matches!(id.as_str(), "request-disconnect" | "disconnect") { rgb(0xf5f0e8) } else { theme.text() })
                    .when(key == "connect" && matches!(id.as_str(), "request-disconnect" | "disconnect"), |d| d.bg(rgb(0xb42318)))
                    .when(!enabled, |d| d.opacity(0.4))
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(theme.hover()))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                cx.stop_propagation();
                                view.menu = None;
                                if key == "connect" {
                                    if let Ok(command) = serde_json::from_value(serde_json::Value::String(id.clone())) {
                                        view.act(Action::Connect { command });
                                    }
                                } else if let Some(value) = menu_setting_value(key, &id) {
                                    view.set(key, value);
                                }
                                cx.notify();
                            }))
                    })
                    .child(
                        div()
                            .w(px(14.0))
                            .flex_none()
                            .when(chosen, |d| d.child(icon(crate::capture::desktop::icons::CHECK, 12.0, theme.text()))),
                    )
                    .child(title.clone()),
            );
        }
        // The catcher sits under the list, so a press on a choice doesn't close the menu first.
        Some(
            div()
                .id("settings-menu-layer")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .child(
                    div()
                        .id("settings-menu-catcher")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|view, _, _, cx| {
                                view.menu = None;
                                cx.notify();
                            }),
                        ),
                )
                .child(list),
        )
    }
}
