use super::*;
use crate::localization::text;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    remote: String,
    branch: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let upstream = format!("{remote}/{branch}");
    let placeholder = text(cx, "branch-name", "本機分支名稱");
    if this.create_branch_input.read(cx).placeholder() != placeholder {
        this.create_branch_input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
    }
    let can_submit = this.can_submit_checkout_remote_branch(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(540.0))
        .child(popover_title(
            theme,
            text(cx, "Checkout remote branch", "切換到遠端分支"),
        ))
        .child(super::popover_rule(theme))
        .child(super::popover_detail(
            theme,
            format!("{} {upstream}", text(cx, "Remote branch:", "遠端分支：")),
        ))
        .child(input_label(
            theme,
            text(cx, "Local branch name", "本機分支名稱"),
        ))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.create_branch_input.clone()),
        )
        .child(super::popover_rule(theme))
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button_labeled(
                        "checkout_remote_branch_cancel",
                        "checkout_remote_branch_cancel_hint",
                        text(cx, "Cancel", "取消"),
                        theme,
                    )
                    .focus_handle(this.checkout_remote_branch_focus.cancel.clone())
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new(
                        "checkout_remote_branch_go",
                        text(cx, "Checkout", "切換"),
                    )
                    .focus_handle(this.checkout_remote_branch_focus.submit.clone())
                    .disabled(!can_submit)
                    .separated_end_slot(super::hotkey_hint(
                        theme,
                        "checkout_remote_branch_go_hint",
                        "Enter",
                    ))
                    .style(components::ButtonStyle::Filled)
                    .on_click(theme, cx, |this, _e, _window, cx| {
                        this.submit_checkout_remote_branch(cx);
                    }),
                ),
        )
}
