use super::*;
use crate::localization::Language;
use gitcomet_core::error::WorktreeRemovalBlocker;
use gitcomet_state::model::ForceRemoveWorktreePrompt;

pub(super) fn panel(
    this: &mut PopoverHost,
    prompt: ForceRemoveWorktreePrompt,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let ForceRemoveWorktreePrompt {
        repo_id,
        path,
        delete_branch,
        blocker,
        ..
    } = prompt;
    let header: SharedString = if delete_branch.is_some() {
        language
            .text(
                "Delete worktree and branch anyway?",
                "仍要刪除工作樹與分支？",
            )
            .into()
    } else {
        language
            .text("Delete worktree anyway?", "仍要刪除工作樹？")
            .into()
    };
    let reason = match blocker {
        WorktreeRemovalBlocker::Changes => language.text(
            "This worktree has modified or untracked files.",
            "此工作樹含有已修改或未追蹤的檔案。",
        ),
        WorktreeRemovalBlocker::Submodules => language.text(
            "This worktree has initialized submodules, so Git removes it only when forced. Uncommitted changes and any submodule commits that exist only in this worktree are lost.",
            "此工作樹含有已初始化的子模組，因此 Git 只能強制移除。未提交的變更與只存在於此工作樹中的子模組提交都會遺失。",
        ),
    };
    let description: SharedString = match delete_branch.as_ref() {
        Some(branch) => {
            let name = crate::view::product_name();
            match language {
                Language::English => format!(
                    "{reason} {name} will force-remove it, then delete the local branch '{branch}'."
                ),
                Language::TraditionalChinese => {
                    format!("{reason}{name} 會強制移除工作樹，再刪除本機分支「{branch}」。")
                }
            }
            .into()
        }
        None => reason.into(),
    };

    let finish_busy = this.state.worktree_finish_protects_worktree(repo_id, &path)
        || this.state.worktree_finish_blocks_workdir_mutation(repo_id)
        || delete_branch
            .as_deref()
            .is_some_and(|b| this.state.worktree_finish_protects_branch(repo_id, b));

    ConfirmDialog::new(header, DIALOG_460_WIDTH)
        .text(theme, description)
        .mono_value(theme, path.display().to_string())
        .command(
            theme,
            format!("git worktree remove --force {}", path.display()),
        )
        .render(
            theme,
            dialog_cancel_button(
                "force_remove_worktree_cancel",
                "force_remove_worktree_cancel_hint",
                theme,
                cx,
            ),
            components::Button::new(
                "force_remove_worktree_go",
                language.text("Delete anyway", "仍要刪除"),
            )
            .style(components::ButtonStyle::Danger)
            .disabled(finish_busy)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                if this.state.worktree_finish_protects_worktree(repo_id, &path)
                    || this.state.worktree_finish_blocks_workdir_mutation(repo_id)
                    || delete_branch
                        .as_deref()
                        .is_some_and(|b| this.state.worktree_finish_protects_branch(repo_id, b))
                {
                    cx.notify();
                    return;
                }
                this.store.dispatch(Msg::ForceRemoveWorktree {
                    repo_id,
                    path: path.clone(),
                    delete_branch: delete_branch.clone(),
                });
                this.close_resolved_popover(cx);
            }),
            cx,
        )
}
