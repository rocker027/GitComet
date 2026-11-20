use super::*;
use crate::kit::interaction as controls;
use crate::localization::Language;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    path: std::path::PathBuf,
    branch: Option<String>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let delete_branch = this.worktree_remove_delete_branch;

    let mut dialog = ConfirmDialog::new(
        language.text("Remove worktree", "移除工作樹"),
        DIALOG_420_WIDTH,
    )
    .text(theme, path.display().to_string());
    if let Some(branch) = branch.as_ref() {
        dialog = dialog.divider(theme).section(
            div().px_1().pb_1().child(
                checkbox_row(
                    "worktree_remove_delete_branch",
                    "worktree_remove_delete_branch",
                    match language {
                        Language::English => format!("Also delete local branch '{branch}'"),
                        Language::TraditionalChinese => format!("一併刪除本機分支「{branch}」"),
                    },
                    delete_branch,
                    &this.worktree_remove_delete_branch_focus_handle,
                    theme,
                    cx,
                )
                .on_activate(
                    false,
                    controls::ControlActivation::Action,
                    cx.listener(|this, _e: &ClickEvent, _w, cx| {
                        this.worktree_remove_delete_branch = !this.worktree_remove_delete_branch;
                        cx.notify();
                    }),
                ),
            ),
        );
        if delete_branch {
            dialog = dialog.note(
                theme,
                language.text(
                    "The branch is deleted after the worktree. An unmerged branch asks first.",
                    "移除工作樹後會刪除分支。若分支尚未合併，會先詢問你。",
                ),
            );
        }
    }

    let finish_busy = this.state.worktree_finish_protects_worktree(repo_id, &path)
        || this.state.worktree_finish_blocks_workdir_mutation(repo_id)
        || branch
            .as_deref()
            .is_some_and(|b| this.state.worktree_finish_protects_branch(repo_id, b));
    if finish_busy {
        dialog = dialog.note(
            theme,
            language.text(
                "This worktree is currently protected by an active Worktree Finish run.",
                "此工作樹正由進行中的工作樹收尾任務保護，暫時無法移除。",
            ),
        );
    }

    dialog.render(
        theme,
        dialog_cancel_button(
            "worktree_remove_cancel",
            "worktree_remove_cancel_hint",
            theme,
            cx,
        ),
        components::Button::new("worktree_remove_go", language.text("Remove", "移除"))
            .style(components::ButtonStyle::Danger)
            .disabled(finish_busy)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                let delete_branch_selected = branch
                    .clone()
                    .filter(|_| this.worktree_remove_delete_branch);
                if this.state.worktree_finish_protects_worktree(repo_id, &path)
                    || this.state.worktree_finish_blocks_workdir_mutation(repo_id)
                    || branch
                        .as_deref()
                        .is_some_and(|b| this.state.worktree_finish_protects_branch(repo_id, b))
                    || delete_branch_selected
                        .as_deref()
                        .is_some_and(|b| this.state.worktree_finish_protects_branch(repo_id, b))
                {
                    cx.notify();
                    return;
                }
                this.store.dispatch(Msg::RemoveWorktree {
                    repo_id,
                    path: path.clone(),
                    delete_branch: delete_branch_selected,
                });
                this.close_popover(cx);
            }),
        cx,
    )
}
