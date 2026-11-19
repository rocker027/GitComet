use super::*;
use crate::localization::Language;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let can_submit = this.can_submit_remote_add(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(640.0))
        .child(popover_title(
            theme,
            language.text("Add remote", "新增遠端"),
        ))
        .child(super::popover_rule(theme))
        .child(input_label(theme, language.text("Name", "名稱")))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.remote_name_input.clone()),
        )
        .child(input_label(theme, language.text("URL", "網址")))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.remote_url_input.clone()),
        )
        .child(super::popover_rule(theme))
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button_labeled(
                        "add_remote_cancel",
                        "add_remote_cancel_hint",
                        language.text("Cancel", "取消"),
                        theme,
                    )
                    .focus_handle(this.remote_add_focus.cancel.clone())
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("add_remote_go", language.text("Add", "新增"))
                        .focus_handle(this.remote_add_focus.submit.clone())
                        .disabled(!can_submit)
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "add_remote_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_remote_add(cx);
                        }),
                ),
        )
}
