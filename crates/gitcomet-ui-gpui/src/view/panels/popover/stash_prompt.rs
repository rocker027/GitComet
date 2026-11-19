use super::*;
use crate::kit::interaction as controls;
use crate::localization::text;

pub(super) fn panel(this: &mut PopoverHost, cx: &mut gpui::Context<PopoverHost>) -> gpui::Div {
    let theme = this.theme;
    let can_stash = this.can_submit_stash(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);
    let placeholder = text(cx, "Stash message (optional)", "貯藏訊息（選填）");
    if this.stash_message_input.read(cx).placeholder() != placeholder {
        this.stash_message_input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
    }

    div()
        .flex()
        .flex_col()
        .w(scaled_px(420.0))
        .child(popover_title(theme, text(cx, "Create stash", "建立貯藏")))
        .child(super::popover_rule(theme))
        .child(
            div()
                .px_2()
                .py_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.stash_message_input.clone()),
        )
        .child(
            super::checkbox_row(
                "stash_include_untracked_toggle",
                "stash_include_untracked_toggle",
                text(cx, "Include untracked files", "包含未追蹤的檔案"),
                this.stash_include_untracked,
                &this.stash_include_untracked_focus_handle,
                theme,
                cx,
            )
            .on_activate(
                false,
                controls::ControlActivation::Action,
                cx.listener(|this, _e: &ClickEvent, _w, cx| {
                    this.stash_include_untracked = !this.stash_include_untracked;
                    cx.notify();
                }),
            ),
        )
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button_labeled(
                        "stash_cancel",
                        "stash_cancel_hint",
                        text(cx, "Cancel", "取消"),
                        theme,
                    )
                    .focus_handle(this.stash_focus.cancel.clone())
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("stash_go", text(cx, "Stash", "貯藏"))
                        .focus_handle(this.stash_focus.submit.clone())
                        .separated_end_slot(super::hotkey_hint(theme, "stash_go_hint", "Enter"))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_stash)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_stash(window, cx);
                        }),
                ),
        )
}
