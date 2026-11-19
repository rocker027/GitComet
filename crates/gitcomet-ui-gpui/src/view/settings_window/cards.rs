//! Settings category cards. Each card is its own function and only the
//! visible one is built, which keeps the render stack frame small in
//! unoptimized builds.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::localization::{Language, text};

/// What the master switch hides, and why it is on by default.
const HIDDEN_REFS_TOOLTIP: &str = "Refs that coding agents, review systems and Git's own bookkeeping \
     write next to your branches.\nHiding them keeps All branches readable, and new ones no longer \
     rebuild the history index.";

pub(super) fn change_tracking_label(
    cx: &App,
    option: ChangeTrackingView,
    english: &'static str,
) -> &'static str {
    let chinese = match option {
        ChangeTrackingView::Combined => "合併至未暫存",
        ChangeTrackingView::SplitUntracked => "獨立區塊",
    };
    text(cx, english, chinese)
}

pub(super) fn file_list_layout_label(cx: &App, option: FileListLayout) -> &'static str {
    let chinese = match option {
        FileListLayout::Flat => "平面清單",
        FileListLayout::Tree => "樹狀清單",
        FileListLayout::Groups => "分組清單",
    };
    text(cx, option.label(), chinese)
}

pub(super) fn file_list_sort_label(
    cx: &App,
    sort: crate::view::rows::CommitFileSort,
) -> &'static str {
    use crate::view::rows::CommitFileSort::*;
    let chinese = match sort {
        PathAscending => "路徑遞增",
        PathDescending => "路徑遞減",
        FileTypeAscending => "檔案類型遞增",
        FileTypeDescending => "檔案類型遞減",
        EditSizeAscending => "變更行數較少優先",
        EditSizeDescending => "變更行數較多優先",
        Edits => "重複變更優先",
    };
    text(cx, sort.label(), chinese)
}

fn browser_open_target_label(cx: &App, target: crate::app::BrowserOpenTarget) -> &'static str {
    let chinese = match target {
        crate::app::BrowserOpenTarget::ExistingWindow => "目前視窗",
        crate::app::BrowserOpenTarget::NewWindow => "新視窗",
    };
    text(cx, target.label(), chinese)
}

fn branch_names_label(cx: &App, mode: HistoryBranchNamesMode) -> &'static str {
    let chinese = match mode {
        HistoryBranchNamesMode::SeparateColumn => "獨立欄位",
        HistoryBranchNamesMode::Inline => "與提交訊息同列",
    };
    text(cx, mode.settings_label(), chinese)
}

impl SettingsWindowView {
    pub(super) fn category_card(
        &mut self,
        category: SettingsCategory,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        crate::view::perf::settings_page_rendered();
        match category {
            SettingsCategory::General => self.general_card(theme, cx),
            SettingsCategory::Appearance => self.appearance_card(theme, cx),
            SettingsCategory::Workspaces => self.workspaces_card(theme, cx),
            SettingsCategory::SecurityPrivacy => self.security_privacy_card(theme, cx),
            SettingsCategory::Terminal => self.terminal_card(theme, cx),
            SettingsCategory::ChangeTracking => self.change_tracking_card(theme, cx),
            SettingsCategory::Diff => self.diff_card(theme, cx),
            SettingsCategory::FileEditing => self.file_editing_card(theme, cx),
            SettingsCategory::GitLog => self.git_log_card(theme, cx),
            SettingsCategory::Remotes => self.remotes_card(theme, cx),
            SettingsCategory::LargeFiles => self.large_files_card(theme, cx),
            SettingsCategory::Tags => self.tags_card(theme, cx),
            SettingsCategory::Maintenance => self.maintenance_card(theme, cx),
            SettingsCategory::GitExecutable => self.git_executable_card(theme, cx),
            SettingsCategory::Environment => self.environment_card(theme, cx),
            SettingsCategory::Links => self.links_card(theme, cx),
        }
    }

    fn general_card(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let card = self.card(
            "settings_window_general",
            text(cx, "General", "一般"),
            theme,
        );
        let card = self.general_integration_rows(card, theme, cx);
        let card = self.general_date_time_rows(card, theme, cx);
        #[cfg(target_os = "windows")]
        let card = self.graphics_renderer_rows(card, theme, cx);
        card
    }

    fn general_integration_rows(
        &self,
        mut general_card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let external_editor_row = self
            .summary_row(
                "settings_window_external_code_editor",
                text(cx, "External code editor", "外部程式碼編輯器"),
                match self.external_editor_setting.as_ref() {
                    Some(setting) => {
                        crate::external_editor::label_for_setting(Some(setting)).into()
                    }
                    None => text(cx, "None", "無").into(),
                },
                self.expanded_section == Some(SettingsSection::ExternalCodeEditor),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::ExternalCodeEditor, cx);
                }),
            );

        let browser_open_target_row = self
            .summary_row(
                "settings_window_browser_open_target",
                text(cx, "Command-line repository opens", "從命令列開啟儲存庫"),
                browser_open_target_label(cx, self.browser_open_target).into(),
                self.expanded_section == Some(SettingsSection::BrowserOpenTarget),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::BrowserOpenTarget, cx);
                }),
            );

        general_card = general_card.child(self.subsection_heading(
            "settings_window_general_integrations",
            text(cx, "Integrations", "整合"),
            theme,
        ));
        general_card = general_card.child(browser_open_target_row);
        if self.expanded_section == Some(SettingsSection::BrowserOpenTarget) {
            let mut detail =
                self.detail_container("settings_window_browser_open_target_container", theme);
            for target in crate::app::BrowserOpenTarget::ALL {
                let description = match (Language::current(cx), target) {
                    (Language::English, _) => target.detail(),
                    (
                        Language::TraditionalChinese,
                        crate::app::BrowserOpenTarget::ExistingWindow,
                    ) => format!(
                        "在目前的 {} 視窗開啟儲存庫。",
                        gitcomet_core::identity::current().display_name()
                    ),
                    (Language::TraditionalChinese, crate::app::BrowserOpenTarget::NewWindow) => {
                        "在新視窗開啟儲存庫。".to_string()
                    }
                };
                detail = detail.child(
                    self.option_row(
                        format!("settings_window_browser_open_target_{}", target.key()),
                        browser_open_target_label(cx, target),
                        Some(description.into()),
                        self.browser_open_target == target,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_browser_open_target(target, cx);
                        }),
                    ),
                );
            }
            general_card = general_card.child(detail);
        }

        general_card = general_card.child(external_editor_row);
        if self.expanded_section == Some(SettingsSection::ExternalCodeEditor) {
            let (item_count, list) = if self.external_editor_options_loading() {
                (
                    1,
                    self.empty_dropdown_list(
                        text(
                            cx,
                            "Detecting installed editors…",
                            "正在偵測已安裝的編輯器…",
                        ),
                        theme,
                    ),
                )
            } else {
                (
                    self.external_editor_options.len(),
                    uniform_list(
                        "settings_window_external_code_editor_list",
                        self.external_editor_options.len(),
                        cx.processor(Self::render_external_editor_option_rows),
                    )
                    .w_full()
                    .min_w(px(0.0))
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(&self.external_editor_scroll)
                    .on_scroll_wheel({
                        let scroll = self.external_editor_scroll.clone();
                        move |event, window, cx| {
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                                cx.stop_propagation();
                            }
                        }
                    })
                    .into_any_element(),
                )
            };
            general_card = general_card.child(self.dropdown_list_container(
                "settings_window_external_code_editor_list_container",
                "settings_window_external_code_editor_scrollbar",
                self.external_editor_scroll.clone(),
                item_count,
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        if self.external_editor_is_custom() {
            let browse_button = components::Button::new(
                "settings_window_external_code_editor_browse",
                text(cx, "Browse", "瀏覽"),
            )
            .style(components::ButtonStyle::Outlined)
            .on_click(theme, cx, |this, _e, window, cx| {
                let view = cx.weak_entity();
                let directory = crate::view::path_prompt::folder_of_existing_entry(
                    this.external_editor_custom_path_input.read(cx).text(),
                );
                let rx = cx.prompt_for_paths(custom_external_editor_path_prompt_options(directory));

                window
                    .spawn(cx, async move |cx| {
                        let result = rx.await;
                        let paths = match result {
                            Ok(Ok(Some(paths))) => paths,
                            Ok(Ok(None)) => return,
                            Ok(Err(_)) | Err(_) => return,
                        };
                        let Some(path) = paths.into_iter().next() else {
                            return;
                        };
                        let _ = view.update(cx, |this, cx| {
                            this.apply_browsed_external_editor_path(path, cx);
                        });
                    })
                    .detach();
            });

            general_card = general_card.child(
                self.detail_container(
                    "settings_window_external_code_editor_custom_container",
                    theme,
                )
                .child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(text(cx, "Custom editor executable", "自訂編輯器的執行檔")),
                )
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .w_full()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(self.external_editor_custom_path_input.clone()),
                        )
                        .child(browse_button),
                )
                .child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(text(cx, "Arguments", "引數")),
                )
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .w_full()
                        .min_w(px(0.0))
                        .child(self.external_editor_custom_arguments_input.clone()),
                ),
            );
        }
        general_card
    }

    fn general_date_time_rows(
        &self,
        mut general_card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let date_format_row = self
            .summary_row(
                "settings_window_date_format",
                text(cx, "Date format", "日期格式"),
                self.date_time_format.label().into(),
                self.expanded_section == Some(SettingsSection::DateFormat),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::DateFormat, cx);
                }),
            );

        let timezone_row = self
            .summary_row(
                "settings_window_timezone",
                text(cx, "Date timezone", "日期時區"),
                self.timezone.label().into(),
                self.expanded_section == Some(SettingsSection::Timezone),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::Timezone, cx);
                }),
            );

        let show_timezone_row = self
            .toggle_row(
                "settings_window_show_timezone",
                text(cx, "Show timezone", "顯示時區"),
                self.show_timezone,
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_show_timezone(!this.show_timezone, cx);
                }),
            );

        general_card = general_card
            .child(self.subsection_heading(
                "settings_window_general_date_time",
                text(cx, "Date & Time", "日期與時間"),
                theme,
            ))
            .child(date_format_row);
        if self.expanded_section == Some(SettingsSection::DateFormat) {
            let list = uniform_list(
                "settings_window_date_format_list",
                DateTimeFormat::all().len(),
                cx.processor(Self::render_date_format_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.date_format_scroll)
            .on_scroll_wheel({
                let scroll = self.date_format_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            general_card = general_card.child(self.dropdown_list_container(
                "settings_window_date_format_list_container",
                "settings_window_date_format_scrollbar",
                self.date_format_scroll.clone(),
                DateTimeFormat::all().len(),
                SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_COMPACT_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        general_card = general_card.child(timezone_row);
        if self.expanded_section == Some(SettingsSection::Timezone) {
            let list = uniform_list(
                "settings_window_timezone_list",
                Timezone::all().len(),
                cx.processor(Self::render_timezone_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.timezone_scroll)
            .on_scroll_wheel({
                let scroll = self.timezone_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            general_card = general_card.child(self.dropdown_list_container(
                "settings_window_timezone_list_container",
                "settings_window_timezone_scrollbar",
                self.timezone_scroll.clone(),
                Timezone::all().len(),
                SETTINGS_DROPDOWN_DENSE_DETAIL_ROW_HEIGHT_PX,
                0.0,
                list,
                theme,
            ));
        }

        general_card = general_card.child(show_timezone_row);
        general_card
    }

    fn terminal_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let terminal_external_row = self
            .summary_row(
                "settings_window_terminal_external",
                text(cx, "External terminal", "外部終端機"),
                self.terminal_preferences.external_summary().into(),
                self.expanded_section == Some(SettingsSection::TerminalExternal),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::TerminalExternal, cx);
                }),
            );

        let terminal_button_row = self
            .summary_row(
                "settings_window_terminal_button",
                text(cx, "Terminal button opens", "終端機按鈕開啟"),
                self.terminal_preferences
                    .terminal_button_target
                    .label()
                    .into(),
                self.expanded_section == Some(SettingsSection::TerminalButton),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::TerminalButton, cx);
                }),
            );

        let mut terminal_card = self.card(
            "settings_window_terminal_card",
            text(cx, "Terminal", "終端機"),
            theme,
        );

        terminal_card = terminal_card.child(terminal_external_row);
        if self.expanded_section == Some(SettingsSection::TerminalExternal) {
            terminal_card = terminal_card
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(
                            "System default is best effort. Use a custom launcher for predictable cross-platform behavior.",
                        ),
                )
                .child(
                    div()
                        .px_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            self.option_row(
                                "settings_window_terminal_external_default",
                                ExternalTerminalMode::SystemDefault.label(),
                                Some("Use the platform default when possible".into()),
                                self.terminal_preferences.external_terminal_mode
                                    == ExternalTerminalMode::SystemDefault,
                                theme,
                            )
                            .on_activate(false, controls::ControlActivation::Action, cx.listener(
                                |this, _e: &ClickEvent, _window, cx| {
                                    this.set_external_terminal_mode(
                                        ExternalTerminalMode::SystemDefault,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .child(
                            self.option_row(
                                "settings_window_terminal_external_custom",
                                ExternalTerminalMode::CustomProgram.label(),
                                Some("Choose a launcher and explicit arguments".into()),
                                self.terminal_preferences.external_terminal_mode
                                    == ExternalTerminalMode::CustomProgram,
                                theme,
                            )
                            .on_activate(false, controls::ControlActivation::Action, cx.listener(
                                |this, _e: &ClickEvent, _window, cx| {
                                    this.set_external_terminal_mode(
                                        ExternalTerminalMode::CustomProgram,
                                        cx,
                                    );
                                },
                            )),
                        ),
                );

            if self.terminal_preferences.external_terminal_mode
                == ExternalTerminalMode::CustomProgram
            {
                terminal_card = terminal_card
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(text(cx, "Program", "程式")),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .w_full()
                            .min_w(px(0.0))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(self.terminal_external_program_input.clone()),
                            )
                            .child(
                                components::Button::new(
                                    "settings_window_terminal_external_browse",
                                    text(cx, "Browse", "瀏覽"),
                                )
                                .style(components::ButtonStyle::Outlined)
                                .on_click(
                                    theme,
                                    cx,
                                    |this, _e, window, cx| {
                                        this.browse_terminal_program_input(
                                            TerminalProgramInputTarget::ExternalTerminal,
                                            window,
                                            cx,
                                        );
                                    },
                                ),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(text(cx, "Arguments", "引數")),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .w_full()
                            .min_w(px(0.0))
                            .child(self.terminal_external_args_input.clone()),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(
                                "One argument per line. Use {cwd} and {repo_name} placeholders.",
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                components::Button::new(
                                    "settings_window_terminal_external_save",
                                    text(cx, "Save", "儲存"),
                                )
                                .style(components::ButtonStyle::Filled)
                                .on_click(
                                    theme,
                                    cx,
                                    |this, _e, _w, cx| {
                                        this.save_terminal_external_draft(cx);
                                    },
                                ),
                            )
                            .child(
                                components::Button::new(
                                    "settings_window_terminal_external_reset",
                                    text(cx, "Reset", "重設"),
                                )
                                .style(components::ButtonStyle::Outlined)
                                .on_click(
                                    theme,
                                    cx,
                                    |this, _e, _w, cx| {
                                        this.reset_terminal_external_draft(cx);
                                    },
                                ),
                            )
                            .child(
                                components::Button::new(
                                    "settings_window_terminal_external_test",
                                    text(cx, "Test launch", "測試啟動"),
                                )
                                .style(components::ButtonStyle::Outlined)
                                .on_click(
                                    theme,
                                    cx,
                                    |this, _e, _w, cx| {
                                        this.test_terminal_launch_from_draft(cx);
                                    },
                                ),
                            ),
                    );
            }
        }

        terminal_card = terminal_card.child(terminal_button_row);
        if self.expanded_section == Some(SettingsSection::TerminalButton) {
            terminal_card = terminal_card
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(
                            text(cx, "Choose what the terminal button opens. Global shortcuts for each can be configured separately.", "選擇終端機按鈕要開啟的項目。可分別設定各項目的全域快捷鍵。"),
                        ),
                )
                .child(
                    div()
                        .px_2()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            self.option_row(
                                "settings_window_terminal_button_embedded",
                                TerminalButtonTarget::Embedded.label(),
                                Some(text(cx, "Toggle the embedded terminal panel", "切換內嵌終端機面板").into()),
                                self.terminal_preferences.terminal_button_target
                                    == TerminalButtonTarget::Embedded,
                                theme,
                            )
                            .on_activate(false, controls::ControlActivation::Action, cx.listener(
                                |this, _e: &ClickEvent, _window, cx| {
                                    this.set_terminal_button_target(
                                        TerminalButtonTarget::Embedded,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .child(
                            self.option_row(
                                "settings_window_terminal_button_external",
                                TerminalButtonTarget::External.label(),
                                Some(text(cx, "Launch the external terminal", "啟動外部終端機").into()),
                                self.terminal_preferences.terminal_button_target
                                    == TerminalButtonTarget::External,
                                theme,
                            )
                            .on_activate(false, controls::ControlActivation::Action, cx.listener(
                                |this, _e: &ClickEvent, _window, cx| {
                                    this.set_terminal_button_target(
                                        TerminalButtonTarget::External,
                                        cx,
                                    );
                                },
                            )),
                        ),
                );
        }

        if let Some(status) = self.terminal_status.clone() {
            terminal_card = terminal_card.child(
                div()
                    .px_2()
                    .pt_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(if status.is_error {
                        theme.colors.status.danger.foreground
                    } else {
                        theme.colors.status.success.foreground
                    })
                    .child(status.text),
            );
        }
        terminal_card
    }

    fn security_privacy_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let allowed_remote_protocols_row = self
            .summary_row(
                "settings_window_allowed_remote_protocols",
                text(cx, "Allowed remote protocols", "允許的遠端通訊協定"),
                remote_url_policy_settings_label(self.remote_url_policy).into(),
                self.expanded_section == Some(SettingsSection::AllowedRemoteProtocols),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::AllowedRemoteProtocols, cx);
                }),
            );

        let remote_markdown_images_row = self
            .summary_row(
                "settings_window_remote_markdown_images",
                text(cx, "Remote Markdown images", "遠端 Markdown 圖片"),
                self.remote_markdown_image_policy.settings_label().into(),
                self.expanded_section == Some(SettingsSection::RemoteMarkdownImages),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::RemoteMarkdownImages, cx);
                }),
            );

        let update_check_locked = crate::view::update_checks_disabled_by_environment();
        let update_check_effective = self.check_for_updates_on_startup && !update_check_locked;
        let update_check_id = "settings_window_check_for_updates_on_startup";
        let update_check_label = text(
            cx,
            "Automatically check for updates on startup",
            "啟動時自動檢查更新",
        );
        let update_check_row = if update_check_locked {
            self.disabled_toggle_row(
                update_check_id,
                update_check_label,
                update_check_effective,
                theme,
            )
        } else {
            self.toggle_row(
                update_check_id,
                update_check_label,
                update_check_effective,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_check_for_updates_on_startup(!this.check_for_updates_on_startup, cx);
                }),
            )
        }
        .border_color(no_separator);

        let mut security_privacy_card = self
            .card(
                "settings_window_security_privacy_card",
                text(cx, "Security / Privacy", "安全與隱私"),
                theme,
            )
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_3()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        text(cx, "Only selected built-in URL protocols may be passed to Git. Custom remote helpers stay blocked. Local paths and SCP-style SSH locations remain available.", "只有你選取的內建 URL 通訊協定可傳給 Git。自訂遠端輔助程式仍會被封鎖。本機路徑與 SCP 格式的 SSH 位址仍可使用。"),
                    ),
            )
            .child(allowed_remote_protocols_row);

        if self.expanded_section == Some(SettingsSection::AllowedRemoteProtocols) {
            let list = uniform_list(
                "settings_window_remote_protocols_list",
                REMOTE_PROTOCOL_OPTIONS.len(),
                cx.processor(Self::render_remote_protocol_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.remote_protocols_scroll)
            .on_scroll_wheel({
                let scroll = self.remote_protocols_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            security_privacy_card = security_privacy_card.child(self.dropdown_list_container(
                "settings_window_remote_protocols_list_container",
                "settings_window_remote_protocols_scrollbar",
                self.remote_protocols_scroll.clone(),
                REMOTE_PROTOCOL_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        security_privacy_card = security_privacy_card
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_3()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        "Repository-controlled HTTP/HTTPS images can act as tracking pixels and reveal that you opened a document. Safe relative images from the repository continue to load in every mode.",
                    ),
            )
            .child(remote_markdown_images_row);

        if self.expanded_section == Some(SettingsSection::RemoteMarkdownImages) {
            let list = uniform_list(
                "settings_window_remote_markdown_images_list",
                REMOTE_MARKDOWN_IMAGE_OPTIONS.len(),
                cx.processor(Self::render_remote_markdown_image_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.remote_markdown_images_scroll)
            .on_scroll_wheel({
                let scroll = self.remote_markdown_images_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            security_privacy_card = security_privacy_card.child(self.dropdown_list_container(
                "settings_window_remote_markdown_images_list_container",
                "settings_window_remote_markdown_images_scrollbar",
                self.remote_markdown_images_scroll.clone(),
                REMOTE_MARKDOWN_IMAGE_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        // A product without an update source has nothing to toggle.
        if !crate::view::update_checks_available() {
            return security_privacy_card;
        }
        security_privacy_card = security_privacy_card.child(update_check_row);
        if update_check_locked {
            security_privacy_card = security_privacy_card.child(
                div()
                    .id("settings_window_update_check_environment_note")
                    .px_2()
                    .pb_3()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        format!(
                            "Disabled by GITCOMET_NO_UPDATE_CHECK. Remove the environment variable and restart {} to change this setting.",
                            crate::view::product_name()
                        ),
                    ),
            );
        }
        security_privacy_card
    }

    fn change_tracking_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let change_tracking_row = self
            .summary_row(
                "settings_window_change_tracking",
                text(cx, "Untracked files", "未追蹤的檔案"),
                change_tracking_label(
                    cx,
                    self.change_tracking_view,
                    self.change_tracking_view.settings_label(),
                )
                .into(),
                self.expanded_section == Some(SettingsSection::ChangeTracking),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::ChangeTracking, cx);
                }),
            );

        let file_list_layout_row = self
            .summary_row(
                "settings_window_file_list_layout",
                text(cx, "Changed-file lists", "變更檔案清單"),
                file_list_layout_label(cx, self.file_list_layout).into(),
                self.expanded_section == Some(SettingsSection::FileListLayout),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::FileListLayout, cx);
                }),
            );

        let file_list_sort_row = self
            .summary_row(
                "settings_window_file_list_sort",
                text(cx, "Changed-file sort", "變更檔案排序"),
                file_list_sort_label(cx, self.file_list_sort).into(),
                self.expanded_section == Some(SettingsSection::FileListSort),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::FileListSort, cx);
                }),
            );

        let mut change_tracking_card = self
            .card(
                "settings_window_change_tracking_card",
                text(cx, "Change tracking", "變更追蹤"),
                theme,
            )
            .child(change_tracking_row);

        if self.expanded_section == Some(SettingsSection::ChangeTracking) {
            let list = uniform_list(
                "settings_window_change_tracking_list",
                CHANGE_TRACKING_OPTIONS.len(),
                cx.processor(Self::render_change_tracking_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.change_tracking_scroll)
            .on_scroll_wheel({
                let scroll = self.change_tracking_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            change_tracking_card = change_tracking_card.child(self.dropdown_list_container(
                "settings_window_change_tracking_list_container",
                "settings_window_change_tracking_scrollbar",
                self.change_tracking_scroll.clone(),
                CHANGE_TRACKING_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        change_tracking_card = change_tracking_card.child(file_list_layout_row);

        if self.expanded_section == Some(SettingsSection::FileListLayout) {
            let list = uniform_list(
                "settings_window_file_list_layout_list",
                FILE_LIST_LAYOUT_OPTIONS.len(),
                cx.processor(Self::render_file_list_layout_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.file_list_layout_scroll)
            .on_scroll_wheel({
                let scroll = self.file_list_layout_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            change_tracking_card = change_tracking_card.child(self.dropdown_list_container(
                "settings_window_file_list_layout_list_container",
                "settings_window_file_list_layout_scrollbar",
                self.file_list_layout_scroll.clone(),
                FILE_LIST_LAYOUT_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        change_tracking_card = change_tracking_card.child(file_list_sort_row);

        if self.expanded_section == Some(SettingsSection::FileListSort) {
            let list = uniform_list(
                "settings_window_file_list_sort_list",
                FILE_LIST_SORT_OPTIONS.len(),
                cx.processor(Self::render_file_list_sort_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.file_list_sort_scroll)
            .on_scroll_wheel({
                let scroll = self.file_list_sort_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            change_tracking_card = change_tracking_card.child(self.dropdown_list_container(
                "settings_window_file_list_sort_list_container",
                "settings_window_file_list_sort_scrollbar",
                self.file_list_sort_scroll.clone(),
                FILE_LIST_SORT_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }
        change_tracking_card
            .child(
                self.toggle_row(
                    "settings_window_open_next_file_after_last_change",
                    text(cx, "Automatically open the next file when the current one has no changes left", "目前檔案的變更清空後，自動開啟下一個檔案"),
                    self.open_next_file_after_last_change,
                    theme,
                )
                .border_color(no_separator)
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_open_next_file_after_last_change(
                            !this.open_next_file_after_last_change,
                            cx,
                        );
                    }),
                ),
            )
            .child(
                div()
                    .debug_selector(|| {
                        "settings_window_open_next_file_after_last_change_help".to_string()
                    })
                    .px_2()
                    .pb_2()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        text(cx, "When S, U, the gutter buttons, a diff menu or Discard leaves the open file with no changes in its list, the diff opens the next file in that list, or the previous one at the end. When off, the diff stays on the file, or closes if you discarded the whole file. Space and the Stage/Unstage button move to the next file either way.", "使用 S、U、行號旁按鈕、差異選單或捨棄操作，使開啟中的檔案從變更清單移除後，差異檢視會開啟清單中的下一個檔案。若已到清單底部，則會開啟上一個檔案。關閉此選項後，差異檢視會留在原檔案。若捨棄整個檔案的變更，則會關閉差異檢視。使用空白鍵或暫存／取消暫存按鈕時，一律會開啟下一個檔案。"),
                    ),
            )
    }

    fn diff_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let diff_scroll_sync_row = self
            .summary_row(
                "settings_window_diff_scroll_sync",
                text(cx, "Scroll sync", "同步捲動"),
                self.diff_scroll_sync.label().into(),
                self.expanded_section == Some(SettingsSection::Diff),
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::Diff, cx);
                }),
            );

        let diff_content_mode_row = self
            .summary_row(
                "settings_window_diff_content_mode",
                text(cx, "Diff mode", "差異模式"),
                self.diff_content_mode.settings_label().into(),
                self.expanded_section == Some(SettingsSection::DiffContentMode),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::DiffContentMode, cx);
                }),
            );

        let diff_whitespace_mode_row = self
            .toggle_row(
                "settings_window_diff_whitespace_mode",
                text(cx, "Show whitespace changes", "顯示空白字元變更"),
                self.diff_whitespace_mode == DiffWhitespaceMode::Show,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_whitespace_mode(this.diff_whitespace_mode.toggled(), cx);
                }),
            );

        let diff_reveal_whitespace_chars_row = self
            .toggle_row(
                "settings_window_diff_reveal_whitespace_chars",
                text(cx, "Reveal whitespace characters", "標示空白字元"),
                self.diff_reveal_whitespace_chars,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_reveal_whitespace_chars(!this.diff_reveal_whitespace_chars, cx);
                }),
            );

        let diff_word_wrap_row = self
            .toggle_row(
                "settings_window_diff_word_wrap",
                text(cx, "Word wrap", "自動換行"),
                self.diff_word_wrap,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_word_wrap(!this.diff_word_wrap, cx);
                }),
            );

        let diff_tab_size_row = self
            .summary_row(
                "settings_window_diff_tab_size",
                text(cx, "Tab size", "定位字元寬度"),
                format!("{} spaces", self.diff_tab_size).into(),
                self.expanded_section == Some(SettingsSection::DiffTabSize),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::DiffTabSize, cx);
                }),
            );

        let diff_show_line_numbers_row = self
            .toggle_row(
                "settings_window_diff_show_line_numbers",
                text(cx, "Show line numbers", "顯示行號"),
                self.diff_show_line_numbers,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_diff_show_line_numbers(!this.diff_show_line_numbers, cx);
                }),
            );

        let mut diff_card = self
            .card("settings_window_diff_card", text(cx, "Diff", "差異"), theme)
            .child(
                self.summary_row(
                    "settings_window_diff_engine",
                    text(cx, "Diff engine", "差異比對引擎"),
                    self.diff_engine.label().into(),
                    self.expanded_section == Some(SettingsSection::DiffEngine),
                    theme,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_section(SettingsSection::DiffEngine, cx)
                    }),
                ),
            );
        if self.expanded_section == Some(SettingsSection::DiffEngine) {
            for engine in DiffEngine::ALL {
                let detail = match engine {
                    DiffEngine::LineByLine => "Compare changed lines and words",
                    DiffEngine::Difftastic => "Compare syntax; works in Split and Inline views",
                };
                diff_card = diff_card.child(
                    self.setting_option_row(
                        format!("settings_window_diff_engine_{}", engine.key()),
                        engine.label(),
                        Some(detail.into()),
                        self.diff_engine == engine,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.set_diff_engine(engine, cx)
                        }),
                    ),
                );
            }
        }
        diff_card = diff_card.child(diff_content_mode_row);

        if self.expanded_section == Some(SettingsSection::DiffContentMode) {
            let list = uniform_list(
                "settings_window_diff_content_mode_list",
                DIFF_CONTENT_MODE_OPTIONS.len(),
                cx.processor(Self::render_diff_content_mode_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.diff_content_mode_scroll)
            .on_scroll_wheel({
                let scroll = self.diff_content_mode_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            diff_card = diff_card.child(self.dropdown_list_container(
                "settings_window_diff_content_mode_list_container",
                "settings_window_diff_content_mode_scrollbar",
                self.diff_content_mode_scroll.clone(),
                DIFF_CONTENT_MODE_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        let diff_view_mode_row = self
            .summary_row(
                "settings_window_diff_view_mode",
                text(cx, "View mode", "檢視模式"),
                self.diff_view_mode.settings_label().into(),
                self.expanded_section == Some(SettingsSection::DiffViewMode),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::DiffViewMode, cx);
                }),
            );

        diff_card = diff_card.child(diff_view_mode_row);

        if self.expanded_section == Some(SettingsSection::DiffViewMode) {
            let list = uniform_list(
                "settings_window_diff_view_mode_list",
                DIFF_VIEW_MODE_OPTIONS.len(),
                cx.processor(Self::render_diff_view_mode_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.diff_view_mode_scroll)
            .on_scroll_wheel({
                let scroll = self.diff_view_mode_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            diff_card = diff_card.child(self.dropdown_list_container(
                "settings_window_diff_view_mode_list_container",
                "settings_window_diff_view_mode_scrollbar",
                self.diff_view_mode_scroll.clone(),
                DIFF_VIEW_MODE_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        diff_card = diff_card
            .child(diff_whitespace_mode_row)
            .child(diff_reveal_whitespace_chars_row)
            .child(diff_word_wrap_row)
            .child(diff_tab_size_row);

        if self.expanded_section == Some(SettingsSection::DiffTabSize) {
            let list = uniform_list(
                "settings_window_diff_tab_size_list",
                DIFF_TAB_SIZE_OPTIONS.len(),
                cx.processor(Self::render_diff_tab_size_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.diff_tab_size_scroll)
            .on_scroll_wheel({
                let scroll = self.diff_tab_size_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            diff_card = diff_card.child(self.dropdown_list_container(
                "settings_window_diff_tab_size_list_container",
                "settings_window_diff_tab_size_scrollbar",
                self.diff_tab_size_scroll.clone(),
                DIFF_TAB_SIZE_OPTIONS.len(),
                SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_COMPACT_LIST_EXTRA_HEIGHT_PX,
                list,
                theme,
            ));
        }

        diff_card = diff_card.child(diff_show_line_numbers_row);

        diff_card = diff_card.child(diff_scroll_sync_row);

        if self.expanded_section == Some(SettingsSection::Diff) {
            let list = uniform_list(
                "settings_window_diff_scroll_sync_list",
                DIFF_SCROLL_SYNC_OPTIONS.len(),
                cx.processor(Self::render_diff_scroll_sync_option_rows),
            )
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .min_h(px(0.0))
            .track_scroll(&self.diff_scroll_sync_scroll)
            .on_scroll_wheel({
                let scroll = self.diff_scroll_sync_scroll.clone();
                move |event, window, cx| {
                    if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                        cx.stop_propagation();
                    }
                }
            });
            let list = restrict_scroll_to_vertical_axis(list).into_any_element();
            diff_card = diff_card.child(self.dropdown_list_container(
                "settings_window_diff_scroll_sync_list_container",
                "settings_window_diff_scroll_sync_scrollbar",
                self.diff_scroll_sync_scroll.clone(),
                DIFF_SCROLL_SYNC_OPTIONS.len(),
                SETTINGS_DROPDOWN_DETAIL_ROW_HEIGHT_PX,
                SETTINGS_DROPDOWN_DETAIL_LIST_EXTRA_HEIGHT_PX + 18.0,
                list,
                theme,
            ));
        }
        diff_card
    }

    fn file_editing_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self.card(
            "settings_window_file_editing_card",
            text(cx, "File editing", "檔案編輯"),
            theme,
        )
        .child(
            self.toggle_row(
                "settings_window_auto_save_file_edits",
                text(cx, "Auto-save edits", "自動儲存編輯"),
                self.auto_save_file_edits,
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_auto_save_file_edits(!this.auto_save_file_edits, cx);
                }),
            ),
        )
    }

    fn git_log_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        let history_default_mode_row = self
            .summary_row(
                "settings_window_git_log_default_mode",
                text(cx, "Default history mode", "預設歷史顯示模式"),
                crate::view::history_mode::localized_history_mode_label(
                    self.default_history_mode,
                    cx,
                )
                .into(),
                self.expanded_section == Some(SettingsSection::GitLogDefaultMode),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::GitLogDefaultMode, cx);
                }),
            );

        let history_columns_row = self
            .summary_row(
                "settings_window_git_log_columns",
                text(cx, "History columns", "歷史欄位"),
                history_columns_settings_label(
                    cx,
                    self.history_show_graph,
                    self.history_show_author,
                    self.history_show_date,
                    self.history_show_sha,
                ),
                self.expanded_section == Some(SettingsSection::GitLogColumns),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::GitLogColumns, cx);
                }),
            );

        // "Lane", not "chain": what this dims is every lane but the
        // selected commit's own. A merge's second parent sits on a
        // lane of its own and washes out with the rest, so the old
        // label promised an ancestry walk the graph no longer does.
        let highlight_commit_chain_row = self
            .toggle_row(
                "settings_window_git_log_highlight_commit_chain",
                text(
                    cx,
                    "Highlight selected commit lane",
                    "醒目顯示選取提交所在的路徑",
                ),
                self.history_highlight_commit_chain,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_history_highlight_commit_chain(
                        !this.history_highlight_commit_chain,
                        cx,
                    );
                }),
            );

        let files_follow_selected_commit_row = self
            .toggle_row(
                "settings_window_git_log_files_follow_selected_commit",
                text(
                    cx,
                    "Follow selected commit while file browsing",
                    "瀏覽檔案時跟隨選取的提交",
                ),
                self.files_follow_selected_commit,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_files_follow_selected_commit(!this.files_follow_selected_commit, cx);
                }),
            );

        let relative_dates_row = self
            .toggle_row(
                "settings_window_git_log_relative_dates",
                text(
                    cx,
                    "Relative dates in history view",
                    "在歷史檢視中顯示相對時間",
                ),
                self.history_relative_dates,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_history_relative_dates(!this.history_relative_dates, cx);
                }),
            );

        let verify_commit_signatures_row = self
            .toggle_row(
                "settings_window_git_log_verify_signatures",
                text(cx, "Verify commit signatures", "驗證提交簽章"),
                self.history_verify_commit_signatures,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_verify_commit_signatures(!this.history_verify_commit_signatures, cx);
                }),
            );

        let show_history_tags_row = self
            .toggle_row(
                "settings_window_git_log_show_tags",
                text(cx, "Show tags in history view", "在歷史檢視中顯示標籤"),
                self.history_show_tags,
                theme,
            )
            .border_color(no_separator)
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_history_show_tags(!this.history_show_tags, cx);
                }),
            );

        let mut git_log_card = self
            .card(
                "settings_window_git_log_card",
                text(cx, "Git log", "Git 歷史"),
                theme,
            )
            .child(history_default_mode_row);

        if self.expanded_section == Some(SettingsSection::GitLogDefaultMode) {
            let mut mode_container =
                self.detail_container("settings_window_git_log_default_mode_container", theme);
            for spec in crate::view::history_mode::history_mode_ui_specs() {
                let mode = spec.mode;
                mode_container = mode_container.child(
                    self.option_row(
                        spec.settings_row_id,
                        spec.localized_label(cx),
                        Some(spec.localized_description(cx).into()),
                        self.default_history_mode == mode,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_default_history_mode(mode, cx);
                        }),
                    ),
                );
            }
            git_log_card = git_log_card.child(
                mode_container.child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(
                            text(cx, "Applies when opening repositories that do not already have a saved history mode.", "開啟尚未儲存歷史顯示模式的儲存庫時，會套用此設定。"),
                        ),
                ),
            );
        }

        git_log_card = git_log_card.child(
            self.summary_row(
                "settings_window_git_log_branch_names",
                text(cx, "Branch names", "分支名稱"),
                branch_names_label(cx, self.history_branch_names).into(),
                self.expanded_section == Some(SettingsSection::GitLogBranchNames),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::GitLogBranchNames, cx);
                }),
            ),
        );
        if self.expanded_section == Some(SettingsSection::GitLogBranchNames) {
            let mut options =
                self.detail_container("settings_window_git_log_branch_names_container", theme);
            for (mode, id) in [
                (
                    HistoryBranchNamesMode::SeparateColumn,
                    "settings_window_git_log_branch_names_separate",
                ),
                (
                    HistoryBranchNamesMode::Inline,
                    "settings_window_git_log_branch_names_inline",
                ),
            ] {
                options = options.child(
                    self.option_row(
                        id,
                        branch_names_label(cx, mode),
                        None,
                        self.history_branch_names == mode,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_history_branch_names(mode, cx);
                        }),
                    ),
                );
            }
            git_log_card = git_log_card.child(options);
        }

        git_log_card = self.git_log_hidden_refs_rows(git_log_card, theme, cx);
        git_log_card = git_log_card.child(history_columns_row);

        if self.expanded_section == Some(SettingsSection::GitLogColumns) {
            git_log_card = git_log_card.child(
                self.detail_container("settings_window_git_log_columns_container", theme)
                    .child(
                        self.toggle_row(
                            "settings_window_git_log_column_graph",
                            text(cx, "Graph", "提交圖"),
                            self.history_show_graph,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_history_column_preferences(
                                    !this.history_show_graph,
                                    this.history_show_author,
                                    this.history_show_date,
                                    this.history_show_sha,
                                    cx,
                                );
                            }),
                        ),
                    )
                    .child(
                        self.toggle_row(
                            "settings_window_git_log_column_author",
                            text(cx, "Author", "作者"),
                            self.history_show_author,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_history_column_preferences(
                                    this.history_show_graph,
                                    !this.history_show_author,
                                    this.history_show_date,
                                    this.history_show_sha,
                                    cx,
                                );
                            }),
                        ),
                    )
                    .child(
                        self.toggle_row(
                            "settings_window_git_log_column_date",
                            text(cx, "Commit date", "提交日期"),
                            self.history_show_date,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_history_column_preferences(
                                    this.history_show_graph,
                                    this.history_show_author,
                                    !this.history_show_date,
                                    this.history_show_sha,
                                    cx,
                                );
                            }),
                        ),
                    )
                    .child(
                        self.toggle_row(
                            "settings_window_git_log_column_sha",
                            "SHA",
                            self.history_show_sha,
                            theme,
                        )
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.set_history_column_preferences(
                                    this.history_show_graph,
                                    this.history_show_author,
                                    this.history_show_date,
                                    !this.history_show_sha,
                                    cx,
                                );
                            }),
                        ),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(text(
                                cx,
                                "Columns may auto-hide in narrow windows.",
                                "在較窄的視窗中，部分欄位可能會自動隱藏。",
                            )),
                    )
                    .child(
                        self.link_row(
                            "settings_window_git_log_reset_widths",
                            text(cx, "Reset column widths", "重設欄寬"),
                            text(cx, "Reset", "重設").into(),
                            theme,
                        )
                        .border_color(no_separator)
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            cx.listener(|this, _e: &ClickEvent, _window, cx| {
                                this.broadcast_preference(
                                    PreferenceChange::ResetHistoryColumnWidths,
                                    cx,
                                );
                                cx.notify();
                            }),
                        ),
                    ),
            );
        }

        git_log_card = git_log_card.child(
            self.toggle_row(
                "settings_window_git_log_show_graph_gradient",
                text(cx, "Show graph gradient", "顯示提交圖漸層"),
                self.history_show_graph_gradient,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_history_show_graph_gradient(!this.history_show_graph_gradient, cx);
                }),
            ),
        );
        git_log_card = git_log_card.child(highlight_commit_chain_row);
        git_log_card = git_log_card.child(files_follow_selected_commit_row);
        git_log_card = git_log_card.child(relative_dates_row);
        git_log_card = git_log_card.child(verify_commit_signatures_row);
        git_log_card.child(show_history_tags_row)
    }

    /// The "Hidden refs" summary row and, expanded, its presets and custom
    /// namespaces. A preset the product always hides shows locked on.
    fn git_log_hidden_refs_rows(
        &self,
        card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        use gitcomet_state::model::PresetVisibility;

        let presets = gitcomet_core::identity::current().history_ref_presets();
        let settings = &self.hidden_ref_settings;
        let expanded = self.expanded_section == Some(SettingsSection::GitLogHiddenRefs);
        let card = card.child(
            self.summary_row(
                "settings_window_git_log_hidden_refs",
                text(cx, "Hidden refs", "隱藏的參照"),
                hidden_refs_summary(cx, settings, presets, &self.fixed_hidden_refs),
                expanded,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::GitLogHiddenRefs, cx);
                }),
            ),
        );
        if !expanded {
            return card;
        }

        let mut detail = self
            .detail_container("settings_window_git_log_hidden_refs_container", theme)
            .child(
                self.toggle_row(
                    "settings_window_git_log_hidden_refs_enabled",
                    text(cx, "Hide tool and bookkeeping refs", "隱藏工具與內部管理參照"),
                    settings.enabled,
                    theme,
                )
                .gitcomet_tooltip(
                    theme,
                    text(
                        cx,
                        HIDDEN_REFS_TOOLTIP,
                        "程式開發代理、審查系統及 Git 會建立供內部管理使用的參照。\n隱藏這些參照可讓「所有分支」更好閱讀，新參照也不會再觸發重建歷史索引。",
                    )
                    .into(),
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        let mut next = this.hidden_ref_settings.clone();
                        next.enabled = !next.enabled;
                        this.set_hidden_ref_settings(next, cx);
                    }),
                ),
            );
        // Fixed presets stay listed with the switch off: the product still
        // hides them, and the summary counts them.
        for preset in presets {
            let id = format!("settings_window_git_log_hidden_ref_preset_{}", preset.id);
            let (label, description) = if preset.id == "codex-backups" {
                (
                    text(cx, preset.label, "Codex 備份參照"),
                    text(
                        cx,
                        preset.description,
                        "使用 Codex 變基或改寫分支前保留的參照。\n這些參照會讓舊提交與目前的分支同時出現在歷史圖。",
                    ),
                )
            } else {
                (preset.label, preset.description)
            };
            let label = format!("{label} ({})", preset.prefixes.join(", "));
            let tooltip = SharedString::from(description);
            let hidden = match settings.preset_visibility(preset, &self.fixed_hidden_refs) {
                PresetVisibility::Fixed => {
                    detail = detail.child(
                        self.disabled_toggle_row(id, label, true, theme)
                            .gitcomet_tooltip(theme, tooltip),
                    );
                    continue;
                }
                _ if !settings.enabled => continue,
                PresetVisibility::Hidden => true,
                PresetVisibility::Shown => false,
            };
            detail = detail.child(
                self.toggle_row(id, label, hidden, theme)
                    .gitcomet_tooltip(theme, tooltip)
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_hidden_ref_preset(preset, !hidden, cx);
                        }),
                    ),
            );
        }
        if settings.enabled {
            detail = self.hidden_ref_custom_namespace_rows(detail, theme, cx);
        }
        card.child(
            detail.child(
                div()
                    .px_2()
                    .pb_2()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(text(
                        cx,
                        "Refs in these namespaces do not start the All branches walk. \
                         Commits that a shown branch reaches stay visible.",
                        "「所有分支」不會從這些命名空間的參照開始追溯。仍可從已顯示的分支追溯到的提交會繼續顯示。",
                    )),
            ),
        )
    }

    /// The user's own namespaces, then the field that adds one.
    fn hidden_ref_custom_namespace_rows(
        &self,
        mut detail: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        // Keyed by the prefix, so removing one never hands its row state on.
        for prefix in self.hidden_ref_settings.custom_prefixes() {
            let id = format!(
                "settings_window_git_log_hidden_ref_custom_{}",
                prefix.as_str()
            );
            let remove_prefix = prefix.clone();
            detail = detail.child(
                self.info_row(
                    id.clone(),
                    prefix.as_str().to_owned(),
                    components::Button::new(format!("{id}_remove"), text(cx, "Remove", "移除"))
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _window, cx| {
                            this.remove_hidden_ref_prefix(&remove_prefix, cx);
                        }),
                    theme,
                ),
            );
        }
        detail = detail.child(
            div()
                .px_2()
                .pt_1()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(self.hidden_ref_prefix_input.clone()),
                )
                .child(
                    components::Button::new(
                        "settings_window_git_log_hidden_ref_add",
                        text(cx, "Add", "新增"),
                    )
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, |this, _e, _window, cx| {
                        this.add_hidden_ref_prefix(cx);
                    }),
                ),
        );
        if let Some(error) = self.hidden_ref_prefix_error {
            detail = detail.child(self.field_error(
                "settings_window_git_log_hidden_ref_error",
                error.to_string(),
                theme,
            ));
        }
        detail
    }

    fn remotes_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self
            .card("settings_window_remotes_card", text(cx, "Remotes", "遠端"), theme)
            .child(
                self.toggle_row(
                    "settings_window_auto_fetch_enabled",
                    text(cx, "Automatically fetch repositories", "自動擷取儲存庫更新"),
                    self.auto_fetch.enabled,
                    theme,
                )
                .on_activate(false, controls::ControlActivation::Action, cx.listener(
                    |this, _e: &ClickEvent, _window, cx| {
                        this.set_auto_fetch_enabled(!this.auto_fetch.enabled, cx);
                    },
                )),
            )
            .child(self.info_row(
                "settings_window_auto_fetch_interval",
                text(cx, "Fetch interval (minutes)", "擷取間隔（分鐘）"),
                div()
                    .w(ui_scale::design_px(AUTO_FETCH_INTERVAL_INPUT_WIDTH_PX, cx))
                    .child(self.auto_fetch_interval_input.clone()),
                theme,
            ))
            .when_some(self.auto_fetch_interval_error, |card, error| {
                card.child(self.field_error(
                    "settings_window_auto_fetch_interval_error",
                    error,
                    theme,
                ))
            })
            .child(
                div()
                    .debug_selector(|| "settings_window_auto_fetch_help".to_string())
                    .px_2()
                    .pb_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .children(auto_fetch_help()),
            )
            .child(
                self.toggle_row(
                    "settings_window_prune_deleted_remote_branches",
                    text(cx, "Prune deleted remote branches during Fetch All", "擷取全部更新時清除已刪除的遠端分支參照"),
                    self.prune_deleted_remote_branches_on_fetch,
                    theme,
                )
                .border_color(no_separator)
                .on_activate(false, controls::ControlActivation::Action, cx.listener(
                    |this, _e: &ClickEvent, _window, cx| {
                        this.set_prune_deleted_remote_branches_on_fetch(
                            !this.prune_deleted_remote_branches_on_fetch,
                            cx,
                        );
                    },
                )),
            )
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        "Fetch All unlinks local branches whose fetched upstream was deleted, but preserves local branches and tags. Pull fetches only its upstream; Pull into current fetches only the selected branch. Neither prunes remote branches.",
                    ),
            )
            .child(
                self.toggle_row(
                    "settings_window_auto_fetch_tags",
                    text(cx, "Automatically fetch tags", "自動擷取標籤"),
                    self.history_tag_fetch_mode == GitLogTagFetchMode::OnRepositoryActivation,
                    theme,
                )
                .border_color(no_separator)
                .on_activate(false, controls::ControlActivation::Action, cx.listener(
                    |this, _e: &ClickEvent, _window, cx| {
                        this.set_history_tag_fetch_mode(
                            match this.history_tag_fetch_mode {
                                GitLogTagFetchMode::OnRepositoryActivation => {
                                    GitLogTagFetchMode::Disabled
                                }
                                GitLogTagFetchMode::Disabled => {
                                    GitLogTagFetchMode::OnRepositoryActivation
                                }
                            },
                            cx,
                        );
                    },
                )),
            )
            .child(
                div()
                    .debug_selector(|| "settings_window_auto_fetch_tags_help".to_string())
                    .px_2()
                    .pb_2()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(
                        "When a repository becomes active, ask each of its remotes which tags it has, so tag menus show whether a tag is pushed. This contacts every remote of the repository.",
                    ),
            )
    }

    fn large_files_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let large_files = self.large_file_settings;
        let large_file_toggle =
            |id: &'static str,
             label: &'static str,
             enabled: bool,
             update: fn(&mut gitcomet_state::model::LargeFileSettings)| {
                self.toggle_row(id, label, enabled, theme).on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        let mut next = this.large_file_settings;
                        update(&mut next);
                        this.set_large_file_settings(next, cx);
                    }),
                )
            };
        let large_file_note = |text: &'static str| {
            div()
                .px_2()
                .pb_2()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(text)
        };
        self.card("settings_window_large_files_card", text(cx, "Large files", "大型檔案"), theme)
            .child(large_file_toggle(
                "settings_window_annex_hide_refs",
                text(cx, "Hide git-annex bookkeeping branches", "隱藏 git-annex 內部管理分支"),
                large_files.hide_annex_refs,
                |s| s.hide_annex_refs = !s.hide_annex_refs,
            ))
            .child(large_file_note(
                "The git-annex branch and synced/* branches are managed by git-annex itself.",
            ))
            .child(large_file_toggle(
                "settings_window_annex_pull_push",
                text(cx, "Use git-annex pull and push on adjusted branches", "在調整分支上使用 git-annex pull 和 push"),
                large_files.annex_pull_push,
                |s| s.annex_pull_push = !s.annex_pull_push,
            ))
            .child(large_file_note(
                "A plain merge into an adjusted branch commits the adjusted files to the wrong branch; git-annex propagates changes to the base branch instead.",
            ))
            .child(large_file_toggle(
                "settings_window_annex_sync_content",
                "git-annex pull, push and sync also move file content",
                large_files.annex_sync_content,
                |s| s.annex_sync_content = !s.annex_sync_content,
            ))
    }

    fn maintenance_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self.card("settings_window_maintenance_card", text(cx, "Maintenance", "維護"), theme)
            .child(
                self.toggle_row(
                    "settings_window_recommend_maintenance",
                    text(cx, "Recommend repository maintenance", "建議維護儲存庫"),
                    self.recommend_repo_maintenance,
                    theme,
                )
                .border_color(no_separator)
                .on_activate(false, controls::ControlActivation::Action, cx.listener(
                    |this, _e: &ClickEvent, _window, cx| {
                        this.set_recommend_repo_maintenance(
                            !this.recommend_repo_maintenance,
                            cx,
                        );
                    },
                )),
            )
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(format!(
                        "Once a day {name} asks Git whether a repository has collected enough loose objects or packs to slow it down (gc.auto, gc.autoPackLimit) and, if so, offers to run git maintenance. {name} never starts maintenance on its own. To skip one repository, set maintenance.auto to false in its Git config.",
                        name = crate::view::product_name()
                    )),
            )
    }

    fn tags_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self.card("settings_window_tags_card", text(cx, "Tags", "標籤"), theme)
            .child(
                self.setting_option_row(
                    "settings_window_tags_default_lightweight",
                    text(cx, "Lightweight", "輕量標籤"),
                    Some(
                        text(cx, "A simple tag pointing directly to a commit. No message, no GPG signing.", "直接指向一筆提交的簡易標籤。沒有訊息，也沒有 GPG 簽章。")
                            .into(),
                    ),
                    self.default_tag_type == DefaultTagType::Lightweight,
                    theme,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_default_tag_type(DefaultTagType::Lightweight, cx);
                    }),
                ),
            )
            .child(
                self.setting_option_row(
                    "settings_window_tags_default_annotated",
                    text(cx, "Annotated", "附註標籤"),
                    Some(
                        text(cx, "Stores tag author, date, and an optional message. Supports GPG signing.", "儲存標籤的作者、日期及選填訊息。支援 GPG 簽章。")
                            .into(),
                    ),
                    self.default_tag_type == DefaultTagType::Annotated,
                    theme,
                )
                .border_color(no_separator)
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.set_default_tag_type(DefaultTagType::Annotated, cx);
                    }),
                ),
            )
    }

    fn git_executable_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let system_git_row = self
            .setting_option_row(
                "settings_window_git_executable_system",
                text(cx, "System PATH", "系統 PATH"),
                Some(
                    text(
                        cx,
                        "Use the first `git` executable available in the current PATH.",
                        "使用目前 PATH 中找到的第一個 `git` 執行檔。",
                    )
                    .into(),
                ),
                self.git_executable_mode == GitExecutableMode::SystemPath,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_git_executable_mode(GitExecutableMode::SystemPath, cx);
                }),
            );

        let custom_git_row = self
        .setting_option_row(
            "settings_window_git_executable_custom",
            text(cx, "Custom executable", "自訂執行檔"),
            Some(
                "Use a specific Git binary and add its directory when Git resolves helper tools."
                    .into(),
            ),
            self.git_executable_mode == GitExecutableMode::Custom,
            theme,
        )
        .on_activate(false, controls::ControlActivation::Action, cx.listener(|this, _e: &ClickEvent, _window, cx| {
            this.set_git_executable_mode(GitExecutableMode::Custom, cx);
        }));

        let mut git_executable_card = self
            .card(
                "settings_window_git_executable",
                text(cx, "Executables", "執行檔"),
                theme,
            )
            .child(
                div()
                    .id("settings_window_git_executable_scope_note")
                    .px_2()
                    .pb_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(git_executable_scope_note()),
            )
            .child(system_git_row)
            .child(custom_git_row)
            .child(
                components::Button::new(
                    "settings_window_recheck_executables",
                    text(cx, "Recheck", "重新檢查"),
                )
                .style(components::ButtonStyle::Outlined)
                .on_click(theme, cx, |_this, _e, _window, cx| {
                    super::super::runtime_probe::request(cx, true);
                }),
            );

        if self.git_executable_mode == GitExecutableMode::Custom {
            let browse_button = components::Button::new(
                "settings_window_git_executable_browse",
                text(cx, "Browse", "瀏覽"),
            )
            .style(components::ButtonStyle::Outlined)
            .on_click(theme, cx, |this, _e, window, cx| {
                let view = cx.weak_entity();
                let directory = crate::view::path_prompt::folder_of_existing_entry(
                    this.git_executable_input.read(cx).text(),
                );
                let rx = cx.prompt_for_paths(gpui::PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: Some(text(cx, "Select Git executable", "選取 Git 執行檔").into()),
                    directory,
                });

                window
                    .spawn(cx, async move |cx| {
                        let result = rx.await;
                        let paths = match result {
                            Ok(Ok(Some(paths))) => paths,
                            Ok(Ok(None)) => return,
                            Ok(Err(_)) | Err(_) => return,
                        };
                        let Some(path) = paths.into_iter().next() else {
                            return;
                        };
                        let _ = view.update(cx, |this, cx| {
                            let next = path.display().to_string();
                            this.git_custom_path_draft = next.clone();
                            this.git_executable_input
                                .update(cx, |input, cx| input.set_text(next, cx));
                            this.apply_git_executable_settings(cx);
                        });
                    })
                    .detach();
            });

            let use_path_button = components::Button::new(
                "settings_window_git_executable_apply",
                text(cx, "Use Path", "使用路徑"),
            )
            .style(components::ButtonStyle::Filled)
            .on_click(theme, cx, |this, _e, _window, cx| {
                this.apply_git_executable_settings(cx);
            });

            git_executable_card = git_executable_card.child(
                self.detail_container("settings_window_git_executable_custom_container", theme)
                    .child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(text(cx, "Custom Git executable", "自訂 Git 執行檔")),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .w_full()
                            .min_w(px(0.0))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(self.git_executable_input.clone()),
                            )
                            .child(browse_button)
                            .child(use_path_button),
                    )
                    .child(
                        div()
                            .px_2()
                            .pb_1()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(text(
                                cx,
                                "Press Enter after editing the path to apply it immediately.",
                                "編輯路徑後按 Enter 即可立即套用。",
                            )),
                    ),
            );
        }

        git_executable_card = git_executable_card.child(self.git_runtime_row(theme));

        if let Some(detail) = self.runtime_info.git.detail.clone() {
            git_executable_card = git_executable_card.child(
                div()
                    .id("settings_window_git_runtime_detail")
                    .px_2()
                    .pb_1()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(detail),
            );
        }

        let signing_tools = self.runtime_info.signing_tools.as_ref();
        let large_file_tools = self.runtime_info.large_file_tools.as_ref();
        for (row_id, detail_id, label, description, info) in [
            (
                "settings_window_gpg_runtime",
                "settings_window_gpg_runtime_detail",
                "GPG",
                GPG_DESCRIPTION,
                gpg_info(signing_tools),
            ),
            (
                "settings_window_ssh_keygen_runtime",
                "settings_window_ssh_keygen_runtime_detail",
                "ssh-keygen",
                SSH_KEYGEN_DESCRIPTION,
                ssh_keygen_info(signing_tools),
            ),
            (
                "settings_window_git_lfs_runtime",
                "settings_window_git_lfs_runtime_detail",
                "Git LFS",
                GIT_LFS_DESCRIPTION,
                git_lfs_info(large_file_tools),
            ),
            (
                "settings_window_git_annex_runtime",
                "settings_window_git_annex_runtime_detail",
                "git-annex",
                GIT_ANNEX_DESCRIPTION,
                git_annex_info(large_file_tools),
            ),
        ] {
            git_executable_card = git_executable_card.child(self.signing_tool_row(
                row_id,
                label,
                description,
                &info,
                theme,
            ));
            if let Some(detail) = info.detail {
                git_executable_card = git_executable_card.child(
                    div()
                        .id(detail_id)
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(detail),
                );
            }
        }

        if let Some(url) = signature_guide_url() {
            git_executable_card = git_executable_card.child(
                self.link_row(
                    "settings_window_signature_guide",
                    text(cx, "Signature verification guide", "簽章驗證指南"),
                    "docs/commit-signatures.md".into(),
                    theme,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    move |_, _, cx| {
                        crate::view::platform_open::open_url_later(&url, cx);
                    },
                ),
            );
        }
        git_executable_card
    }

    fn links_card(&self, theme: AppTheme, cx: &mut gpui::Context<Self>) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        self.card("settings_window_links", text(cx, "Links", "連結"), theme)
            .when_some(themes_guide_url(), |card, url| {
                card.child(
                    self.link_row(
                        "settings_window_links_theme_guide",
                        text(cx, "Theme guide", "佈景主題指南"),
                        "docs/themes.md".into(),
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        move |_, _, cx| {
                            crate::view::platform_open::open_url_later(&url, cx);
                        },
                    ),
                )
            })
            .when_some(repository_url(), |card, url| {
                let label = if url.contains("://github.com/") {
                    "GitHub"
                } else {
                    text(cx, "Source code", "原始碼")
                };
                card.child(
                    self.link_row("settings_window_github", label, url_label(url), theme)
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            move |_, _, cx| {
                                crate::view::platform_open::open_url_later(url, cx);
                            },
                        ),
                )
            })
            .when_some(license_link(), |card, license| {
                card.child(
                    self.link_row(
                        "settings_window_license",
                        text(cx, "License", "授權條款"),
                        license.name.to_string().into(),
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        move |_, _, cx| {
                            crate::view::platform_open::open_url_later(&license.url, cx);
                        },
                    ),
                )
            })
            .when_some(crate::view::editions_url(), |card, url| {
                let host = url_label(url);
                let host: SharedString = host
                    .split(['/', '#'])
                    .next()
                    .unwrap_or_default()
                    .to_string()
                    .into();
                card.child(
                    self.link_row(
                        "settings_window_professional_edition_upgrade",
                        text(cx, "Upgrade to Professional edition", "升級至專業版"),
                        host,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        move |_, _, cx| {
                            crate::view::platform_open::open_url_later(url, cx);
                        },
                    ),
                )
            })
            .child(
                self.link_row(
                    "settings_window_open_source_licenses",
                    text(cx, "Open source licenses", "開放原始碼授權"),
                    text(cx, "Show", "顯示").into(),
                    theme,
                )
                .border_color(no_separator)
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _window, cx| {
                        this.show_open_source_licenses(cx);
                    }),
                ),
            )
    }
}

/// How automatic fetch schedules itself, and what it does to spare remote
/// servers. Kept in step with docs/automatic-fetch.md.
pub(super) fn auto_fetch_help() -> [String; 2] {
    [
        format!(
            "Fetches all remotes at this interval while {} is running, in every open \
             workspace, without pulling. Repositories fetched within the interval, including \
             from the command line, wait until it has passed. Worktrees of one repository \
             share a fetch.",
            crate::view::product_name()
        ),
        format!(
            "To limit load on remote servers, background tabs that find no new commits wait \
             up to {}× the interval, fetches to the same server are spaced out, unreachable \
             servers are retried with growing delays, and a failed sign-in pauses automatic \
             fetching from that server until you use Fetch all. Nothing is fetched while a \
             sign-in dialog is open.",
            gitcomet_state::model::AUTO_FETCH_QUIET_MAX_MULTIPLIER
        ),
    ]
}
