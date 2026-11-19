//! The Appearance page: typography, theme tiles, and interface scale and
//! density. Split into one function per section to keep the unoptimized
//! render frame small.

use super::*;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};

impl SettingsWindowView {
    pub(super) fn appearance_card(
        &mut self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let card = self.card(
            "settings_window_appearance",
            text(cx, "Appearance", "外觀"),
            theme,
        );
        let card = self.appearance_language_rows(card, theme, cx);
        let card = self.appearance_typography_rows(card, theme, cx);
        let card = self.appearance_theme_rows(card, theme, cx);
        self.appearance_interface_rows(card, theme, cx)
    }

    fn appearance_language_rows(
        &self,
        mut card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let selected = Language::current(cx);
        let expanded = self.expanded_section == Some(SettingsSection::Language);
        card = card.child(
            self.summary_row(
                "settings_window_language",
                text(cx, "Language", "語言"),
                selected.label().into(),
                expanded,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _, _, cx| this.toggle_section(SettingsSection::Language, cx)),
            ),
        );
        if expanded {
            let mut options = self.detail_container("settings_window_language_options", theme);
            for language in Language::ALL {
                options = options.child(
                    self.option_row(
                        format!("settings_window_language_{}", language.key()),
                        language.label(),
                        None,
                        selected == language,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _, window, cx| {
                            language.select(cx);
                            window.set_window_title(&settings_window_title(language));
                            this.set_expanded_section(None, cx);
                        }),
                    ),
                );
            }
            card = card.child(options);
        }
        card
    }

    fn appearance_theme_rows(
        &mut self,
        card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let no_separator = gpui::rgba(0x00000000);
        // One read of the themes folder serves every lookup below.
        let themes = crate::theme::ThemeCatalog::load();
        card.child(self.subsection_heading(
            "settings_window_appearance_theme",
            text(cx, "Theme", "佈景主題"),
            theme,
        ))
        .child(self.app_theme_tile_grid(&themes, theme, cx))
        .children(self.workspace_theme_override_row(&themes, theme, cx))
        .child(
            self.detail_container("settings_window_theme_links_container", theme)
                // Above the folder link, so a theme that is missing from the
                // tiles is explained right next to the way to go and fix it.
                .children(self.rejected_theme_rows(&themes.issues, theme))
                .child(
                    self.link_row(
                        "settings_window_theme_custom_folder",
                        text(cx, "Open custom theme folder", "開啟自訂主題資料夾"),
                        self.custom_theme_folder_detail(),
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(|this, _e: &ClickEvent, _window, cx| {
                            this.open_custom_theme_folder(cx);
                        }),
                    ),
                )
                .when_some(themes_guide_url(), |container, url| {
                    container.child(
                        self.link_row(
                            "settings_window_theme_guide",
                            text(cx, "Theme guide", "主題指南"),
                            url.clone().into(),
                            theme,
                        )
                        .border_color(no_separator)
                        .on_activate(
                            false,
                            controls::ControlActivation::Action,
                            move |_, _, cx| {
                                crate::view::platform_open::open_url_later(&url, cx);
                            },
                        ),
                    )
                }),
        )
    }

    fn appearance_interface_rows(
        &mut self,
        mut card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let ui_scale_row = self
            .summary_row(
                "settings_window_ui_scale",
                text(cx, "Default UI scale", "預設介面縮放"),
                ui_scale::label(self.default_ui_scale_percent).into(),
                self.expanded_section == Some(SettingsSection::UiScale),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::UiScale, cx);
                }),
            );

        let window_controls_row = self
            .summary_row(
                "settings_window_window_controls",
                text(cx, "Window controls", "視窗控制項"),
                self.window_controls_mode.label().into(),
                self.expanded_section == Some(SettingsSection::WindowControls),
                theme,
            )
            .border_color(gpui::rgba(0x00000000))
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::WindowControls, cx);
                }),
            );

        card = card.child(self.subsection_heading(
            "settings_window_appearance_interface",
            text(cx, "Interface", "介面"),
            theme,
        ));
        card = card.child(ui_scale_row);
        if self.expanded_section == Some(SettingsSection::UiScale) {
            let mut detail = self.detail_container("settings_window_ui_scale_container", theme);
            for percent in ui_scale::UI_SCALE_PRESETS.iter().copied() {
                let detail_text = match percent {
                    80 | 90 => Some(text(cx, "Fit more on screen", "在畫面上顯示更多內容").into()),
                    110 | 125 | 150 => {
                        Some(text(cx, "Larger controls and text", "放大控制項與文字").into())
                    }
                    _ => None,
                };
                detail = detail.child(
                    self.option_row(
                        format!("settings_window_ui_scale_{percent}"),
                        ui_scale::label(percent),
                        detail_text,
                        self.default_ui_scale_percent == percent,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, window, cx| {
                            this.set_ui_scale_percent(percent, window, cx);
                        }),
                    ),
                );
            }
            card = card.child(
                detail.child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(
                            "New windows open at this scale. Zoom one window with Ctrl/Cmd +, -, \
                             and 0, or the zoom button in its status bar.",
                        ),
                ),
            );
        }

        card = card.child(self.density_control(cx));
        card = card.child(window_controls_row);
        if self.expanded_section == Some(SettingsSection::WindowControls) {
            let mut detail =
                self.detail_container("settings_window_window_controls_container", theme);
            for mode in crate::kit::window_chrome::controls::WindowControlsMode::ALL {
                detail = detail.child(
                    self.option_row(
                        format!("settings_window_window_controls_{}", mode.key()),
                        mode.label(),
                        Some(mode.detail().into()),
                        self.window_controls_mode == mode,
                        theme,
                    )
                    .on_activate(
                        false,
                        controls::ControlActivation::Action,
                        cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                            this.set_window_controls_mode(mode, cx);
                        }),
                    ),
                );
            }
            card = card.child(detail);
        }
        card
    }

    fn appearance_typography_rows(
        &mut self,
        mut card: Stateful<gpui::Div>,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let ui_font_row = self
            .summary_row(
                "settings_window_ui_font",
                text(cx, "UI Font", "介面字型"),
                crate::font_preferences::display_label(&self.ui_font_family).into(),
                self.expanded_section == Some(SettingsSection::UiFont),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::UiFont, cx);
                }),
            );

        let editor_font_row = self
            .summary_row(
                "settings_window_editor_font",
                text(cx, "Editor Font", "編輯器字型"),
                crate::font_preferences::display_label(&self.editor_font_family).into(),
                self.expanded_section == Some(SettingsSection::EditorFont),
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.toggle_section(SettingsSection::EditorFont, cx);
                }),
            );

        let font_ligatures_row = self
            .toggle_row(
                "settings_window_use_font_ligatures",
                text(cx, "Use font ligatures", "使用連字"),
                self.use_font_ligatures,
                theme,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _window, cx| {
                    this.set_use_font_ligatures(!this.use_font_ligatures, cx);
                }),
            );

        card = card.child(self.subsection_heading(
            "settings_window_appearance_typography",
            text(cx, "Typography", "字型"),
            theme,
        ));
        card = card.child(ui_font_row);
        if self.expanded_section == Some(SettingsSection::UiFont) {
            let list = if self.ui_font_options.is_empty() {
                self.empty_dropdown_list(text(cx, "No fonts available.", "沒有可用的字型。"), theme)
            } else {
                restrict_scroll_to_vertical_axis(
                    uniform_list(
                        "settings_window_ui_font_list",
                        self.ui_font_options.len(),
                        cx.processor(Self::render_ui_font_option_rows),
                    )
                    .w_full()
                    .min_w(px(0.0))
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(&self.ui_font_scroll)
                    .on_scroll_wheel({
                        let scroll = self.ui_font_scroll.clone();
                        move |event, window, cx| {
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                                cx.stop_propagation();
                            }
                        }
                    }),
                )
                .into_any_element()
            };
            card = card
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(self.font_options_hint(self.ui_font_family.as_str())),
                )
                .child(self.dropdown_list_container(
                    "settings_window_ui_font_list_container",
                    "settings_window_ui_font_scrollbar",
                    self.ui_font_scroll.clone(),
                    self.ui_font_options.len(),
                    SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                    0.0,
                    list,
                    theme,
                ));
        }

        card = card.child(editor_font_row);
        if self.expanded_section == Some(SettingsSection::EditorFont) {
            let list = if self.editor_font_options.is_empty() {
                self.empty_dropdown_list(text(cx, "No fonts available.", "沒有可用的字型。"), theme)
            } else {
                restrict_scroll_to_vertical_axis(
                    uniform_list(
                        "settings_window_editor_font_list",
                        self.editor_font_options.len(),
                        cx.processor(Self::render_editor_font_option_rows),
                    )
                    .w_full()
                    .min_w(px(0.0))
                    .h_full()
                    .min_h(px(0.0))
                    .track_scroll(&self.editor_font_scroll)
                    .on_scroll_wheel({
                        let scroll = self.editor_font_scroll.clone();
                        move |event, window, cx| {
                            if uniform_list_should_stop_scroll_propagation(&scroll, event, window) {
                                cx.stop_propagation();
                            }
                        }
                    }),
                )
                .into_any_element()
            };
            card = card
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(self.font_options_hint(self.editor_font_family.as_str())),
                )
                .child(self.dropdown_list_container(
                    "settings_window_editor_font_list_container",
                    "settings_window_editor_font_scrollbar",
                    self.editor_font_scroll.clone(),
                    self.editor_font_options.len(),
                    SETTINGS_DROPDOWN_COMPACT_ROW_HEIGHT_PX,
                    0.0,
                    list,
                    theme,
                ));
        }

        card = card.child(font_ligatures_row);
        card.child(self.font_size_controls(cx))
    }
}
