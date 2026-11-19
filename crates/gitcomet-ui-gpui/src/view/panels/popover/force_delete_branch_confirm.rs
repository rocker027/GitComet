use super::*;
use crate::localization::text;

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: gitcomet_state::model::ForceDeleteBranchPrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let gitcomet_state::model::ForceDeleteBranchPrompt { repo_id, name, .. } = prompt;

    ConfirmDialog::new(
        text(cx, "Delete branch anyway?", "仍要刪除分支嗎？"),
        DIALOG_420_WIDTH,
    )
    .mono_value(theme, name.clone())
    .text(
        theme,
        text(
            cx,
            "This will permanently delete the local branch, even if it is not fully merged.",
            "即使尚未完全合併，這仍會永久刪除該本機分支。",
        ),
    )
    .command(theme, format!("git branch -D {name}"))
    .render(
        theme,
        dialog_cancel_button(
            "force_delete_branch_cancel",
            "force_delete_branch_cancel_hint",
            theme,
            cx,
        ),
        components::Button::new(
            "force_delete_branch_go",
            text(cx, "Delete anyway", "仍要刪除"),
        )
        .style(components::ButtonStyle::Danger)
        .on_click(theme, cx, move |this, _e, _w, cx| {
            this.store.dispatch(Msg::ForceDeleteBranch {
                repo_id,
                name: name.clone(),
            });
            this.close_resolved_popover(cx);
        }),
        cx,
    )
}
