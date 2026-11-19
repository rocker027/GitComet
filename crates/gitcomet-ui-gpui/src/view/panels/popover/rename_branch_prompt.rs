use super::*;
use crate::localization::text;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    name: String,
    is_current_branch: bool,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let can_rename = this.can_submit_rename_branch(cx);
    let placeholder = text(cx, "branch-name", "分支名稱");
    if this.create_branch_input.read(cx).placeholder() != placeholder {
        this.create_branch_input
            .update(cx, |input, cx| input.set_placeholder(placeholder, cx));
    }
    let ui_scale_percent = super::popover_ui_scale_percent(cx);
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(540.0))
        .child(popover_title(
            theme,
            if is_current_branch {
                text(cx, "Rename current branch", "重新命名目前的分支")
            } else {
                text(cx, "Rename branch", "重新命名分支")
            },
        ))
        .child(super::popover_rule(theme))
        .child(super::popover_detail(
            theme,
            format!("{} {name}", text(cx, "Current name:", "目前名稱：")),
        ))
        .child(input_label(
            theme,
            text(cx, "New branch name", "新分支名稱"),
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
                        "rename_branch_cancel",
                        "rename_branch_cancel_hint",
                        text(cx, "Cancel", "取消"),
                        theme,
                    )
                    .focus_handle(this.create_branch_from_ref_focus.cancel.clone())
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("rename_branch_go", text(cx, "Rename", "重新命名"))
                        .focus_handle(this.create_branch_from_ref_focus.submit.clone())
                        .separated_end_slot(hotkey_hint(theme, "rename_branch_go_hint", "Enter"))
                        .style(components::ButtonStyle::Filled)
                        .disabled(!can_rename)
                        .on_click(theme, cx, |this, _e, window, cx| {
                            this.submit_rename_branch(window, cx);
                        }),
                ),
        )
}
