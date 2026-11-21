use super::*;
use crate::localization::Language;

pub(super) fn panel(
    this: &mut PopoverHost,
    _repo_id: RepoId,
    name: String,
    kind: RemoteUrlKind,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let title = match kind {
        RemoteUrlKind::Fetch => language.text("Edit remote URL (fetch)", "編輯遠端擷取網址"),
        RemoteUrlKind::Push => language.text("Edit remote URL (push)", "編輯遠端推送網址"),
    };
    let can_submit = this.can_submit_remote_edit_url(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);

    div()
        .flex()
        .flex_col()
        .w(scaled_px(640.0))
        .child(popover_title(theme, title))
        .child(super::popover_rule(theme))
        .child(super::popover_detail(
            theme,
            match language {
                Language::English => format!("Remote: {name}"),
                Language::TraditionalChinese => format!("遠端：{name}"),
            },
        ))
        .child(
            div()
                .px_2()
                .pb_1()
                .w_full()
                .min_w(px(0.0))
                .child(this.remote_url_edit_input.clone()),
        )
        .child(super::popover_rule(theme))
        .child(
            super::prompt_footer_row()
                .child(
                    cancel_button_labeled(
                        "edit_remote_url_cancel",
                        "edit_remote_url_cancel_hint",
                        language.text("Cancel", "取消"),
                        theme,
                    )
                    .focus_handle(this.remote_edit_focus.cancel.clone())
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.dismiss_prompt_popover(window, cx);
                    }),
                )
                .child(
                    components::Button::new("edit_remote_url_go", language.text("Save", "儲存"))
                        .focus_handle(this.remote_edit_focus.submit.clone())
                        .disabled(!can_submit)
                        .separated_end_slot(super::hotkey_hint(
                            theme,
                            "edit_remote_url_go_hint",
                            "Enter",
                        ))
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, |this, _e, _w, cx| {
                            this.submit_remote_edit_url(cx);
                        }),
                ),
        )
}
