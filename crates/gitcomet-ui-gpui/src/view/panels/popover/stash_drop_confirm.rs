use super::*;
use crate::localization::text;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    index: usize,
    message: String,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let reference = format!("stash@{{{index}}}");
    let label = if message.is_empty() {
        reference.clone()
    } else {
        format!("{reference} {message}")
    };

    ConfirmDialog::new(text(cx, "Drop stash?", "要刪除貯藏嗎？"), DIALOG_420_WIDTH)
        .mono_value(theme, label)
        .text(
            theme,
            text(
                cx,
                "This permanently removes this stash entry.",
                "這會永久刪除此貯藏項目。",
            ),
        )
        .command(theme, format!("git stash drop {reference}"))
        .render(
            theme,
            cancel_button_labeled(
                "stash_drop_confirm_cancel",
                "stash_drop_cancel_hint",
                text(cx, "Cancel", "取消"),
                theme,
            )
            .on_click(theme, cx, move |this, _e, _w, cx| {
                this.store.dispatch(Msg::LoadStashes { repo_id });
                this.close_popover(cx);
            }),
            components::Button::new("stash_drop_confirm_go", text(cx, "Drop", "刪除"))
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    this.store.dispatch(Msg::DropStash { repo_id, index });
                    this.store.dispatch(Msg::LoadStashes { repo_id });
                    this.close_popover(cx);
                }),
            cx,
        )
}
