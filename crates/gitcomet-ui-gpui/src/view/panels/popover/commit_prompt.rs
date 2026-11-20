use super::*;
use crate::localization::Language;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let can_commit = this.can_submit_commit_prompt(cx);
    let submit_label = if this
        .active_repo()
        .is_some_and(|repo| stage_all_commit_paths(repo, false).is_ok())
    {
        language.text("Stage All & Commit", "暫存全部並提交")
    } else {
        language.text("Commit", "提交")
    };
    let scope_summary = this
        .active_repo()
        .map(|repo| DetailsPaneView::commit_scope_summary(repo, false, language));
    let scaled_px = super::popover_scaled_px_fn(cx);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title(
            theme,
            language.text("Commit Changes", "提交變更"),
        ))
        .when_some(scope_summary, |panel, summary| {
            panel.child(
                div().px_2().py_2().child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .text_size(theme.ui_text(12.0))
                        .text_color(theme.colors.foreground.secondary)
                        .debug_selector(|| "commit_prompt_scope_summary".to_string())
                        .child(summary),
                ),
            )
        })
        .child(super::popover_rule(theme).debug_selector(|| "commit_prompt_rule".to_owned()))
        .child(
            div()
                .px_2()
                .pt_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.commit_prompt_message_input.clone()),
        )
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button("commit_prompt_cancel", "commit_prompt_cancel_hint", theme)
                        .focus_handle(this.commit_prompt_focus.cancel.clone())
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.dismiss_prompt_popover(window, cx);
                        }),
                )
                .child(
                    components::Button::new("commit_prompt_submit", submit_label)
                        .focus_handle(this.commit_prompt_focus.submit.clone())
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "commit_prompt_submit_hint",
                            crate::kit::components::secondary_shortcut("Enter"),
                        ))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_commit)
                        .on_click(theme, cx, move |this, _e, window, cx| {
                            this.submit_commit_prompt(window, cx);
                        }),
                ),
        )
}
