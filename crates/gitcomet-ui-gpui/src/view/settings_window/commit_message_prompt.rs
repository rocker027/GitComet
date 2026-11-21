use super::*;
use gitcomet_state::store::orca_commit_message::DEFAULT_COMMIT_MESSAGE_PROMPT;

/// The saved template, or the default one when none was saved yet.
pub(super) fn saved_commit_message_prompt() -> String {
    session::load()
        .commit_message_prompt_template
        .filter(|template| !template.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_COMMIT_MESSAGE_PROMPT.to_string())
}

/// Saving the default text stores an empty template, so later default
/// updates still reach this user.
fn persist_commit_message_prompt(template: String, cx: &mut gpui::Context<SettingsWindowView>) {
    let template = if template.trim() == DEFAULT_COMMIT_MESSAGE_PROMPT.trim() {
        String::new()
    } else {
        template
    };
    cx.background_spawn(async move {
        let _ = session::persist_ui_settings(session::UiSettings {
            commit_message_prompt_template: Some(template),
            ..session::UiSettings::default()
        });
    })
    .detach();
}

impl SettingsWindowView {
    pub(super) fn commit_message_prompt_card(
        &self,
        theme: AppTheme,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<gpui::Div> {
        let hint = |label: SharedString| {
            div()
                .px_2()
                .pb_1()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(label)
        };
        self.card(
            "settings_window_commit_message_card",
            text(cx, "AI commit message", "AI 提交訊息"),
            theme,
        )
        .child(hint(
            text(
                cx,
                "The prompt sent to the default Orca agent with the staged diff when you generate a commit message.",
                "產生提交訊息時，這段提示詞會連同已暫存的差異送給 Orca 的預設 agent。",
            )
            .into(),
        ))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(self.commit_message_prompt_input.clone()),
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
                        "settings_window_commit_message_save",
                        text(cx, "Save", "儲存"),
                    )
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.save_commit_message_prompt(cx);
                    }),
                )
                .child(
                    components::Button::new(
                        "settings_window_commit_message_reset",
                        text(cx, "Restore default", "還原預設"),
                    )
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        this.reset_commit_message_prompt(cx);
                    }),
                ),
        )
        .when_some(self.commit_message_prompt_status.clone(), |card, status| {
            card.child(hint(status))
        })
    }

    fn save_commit_message_prompt(&mut self, cx: &mut gpui::Context<Self>) {
        let template = self.commit_message_prompt_input.read(cx).text().to_string();
        persist_commit_message_prompt(template, cx);
        self.commit_message_prompt_status =
            Some(text(cx, "Prompt template saved.", "已儲存提示詞模板。").into());
        cx.notify();
    }

    fn reset_commit_message_prompt(&mut self, cx: &mut gpui::Context<Self>) {
        self.commit_message_prompt_input.update(cx, |input, cx| {
            input.set_text(DEFAULT_COMMIT_MESSAGE_PROMPT, cx);
        });
        persist_commit_message_prompt(String::new(), cx);
        self.commit_message_prompt_status = Some(
            text(
                cx,
                "Default prompt template restored.",
                "已還原預設提示詞模板。",
            )
            .into(),
        );
        cx.notify();
    }
}
