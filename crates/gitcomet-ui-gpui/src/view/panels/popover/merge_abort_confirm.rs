use super::*;
use crate::localization::Language;
use gitcomet_core::services::SequencerState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AbortMode {
    Merge,
    RebaseOrApply,
    CherryPick,
    Revert,
}

fn abort_mode(repo: &RepoState) -> AbortMode {
    if merge_in_progress(repo) {
        return AbortMode::Merge;
    }
    match active_sequencer_state(repo) {
        SequencerState::CherryPick => AbortMode::CherryPick,
        SequencerState::Revert => AbortMode::Revert,
        SequencerState::RebaseOrApply => AbortMode::RebaseOrApply,
        SequencerState::None => AbortMode::Merge,
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let mode = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .map(abort_mode)
        .unwrap_or(AbortMode::Merge);

    let (title, body, command, button_id, button_label) = match mode {
        AbortMode::Merge => (
            language.text("Abort merge?", "要中止合併嗎？"),
            language.text(
                "This will abort the current merge and restore the pre-merge state. Any resolved conflicts will be lost.",
                "這會中止目前的合併並還原到合併前的狀態。已解決的衝突會失去處理結果。",
            ),
            "git merge --abort",
            "merge_abort_go",
            language.text("Abort merge", "中止合併"),
        ),
        AbortMode::RebaseOrApply => (
            language.text("Abort apply/rebase?", "要中止套用修補檔或變基嗎？"),
            language.text(
                "This will abort the in-progress patch apply or rebase and restore the previous state. Any resolved conflicts will be lost.",
                "這會中止進行中的修補檔套用或變基，並還原到先前的狀態。已解決的衝突會失去處理結果。",
            ),
            "git rebase --abort / git am --abort",
            "rebase_or_apply_abort_go",
            language.text("Abort", "中止"),
        ),
        AbortMode::CherryPick => (
            language.text("Abort cherry-pick?", "要中止揀選提交嗎？"),
            language.text(
                "This will abort the current cherry-pick and restore the previous state. Any resolved conflicts will be lost.",
                "這會中止目前的揀選提交操作，並還原到先前的狀態。已解決的衝突會失去處理結果。",
            ),
            "git cherry-pick --abort",
            "cherry_pick_abort_go",
            language.text("Abort cherry-pick", "中止揀選"),
        ),
        AbortMode::Revert => (
            language.text("Abort revert?", "要中止還原提交嗎？"),
            language.text(
                "This will stop the current revert and reset the working tree to HEAD. Any resolved conflicts and staged reverted changes will be lost; commits an earlier step already made stay on the branch.",
                "這會中止目前的還原提交操作，並將工作樹重設到 HEAD。衝突的處理結果與已暫存的還原變更將會遺失。先前步驟已建立的提交會留在分支上。",
            ),
            "git revert --abort",
            "revert_abort_go",
            language.text("Abort revert", "中止還原"),
        ),
    };

    ConfirmDialog::new(title, DIALOG_360_WIDTH)
        .text(theme, body)
        .command(theme, command)
        .render(
            theme,
            dialog_cancel_button("merge_abort_cancel", "merge_abort_cancel_hint", theme, cx),
            components::Button::new(button_id, button_label)
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    match mode {
                        AbortMode::Merge => this.store.dispatch(Msg::MergeAbort { repo_id }),
                        AbortMode::RebaseOrApply | AbortMode::CherryPick | AbortMode::Revert => {
                            this.store.dispatch(Msg::RebaseAbort { repo_id })
                        }
                    }
                    this.close_popover(cx);
                }),
            cx,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::RepoSpec;
    use std::path::PathBuf;

    #[test]
    fn abort_mode_prefers_the_specific_sequencer_over_rebase_in_progress() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        repo.rebase_in_progress = Loadable::Ready(true);
        for (state, expected) in [
            (SequencerState::Revert, AbortMode::Revert),
            (SequencerState::CherryPick, AbortMode::CherryPick),
            (SequencerState::RebaseOrApply, AbortMode::RebaseOrApply),
        ] {
            repo.sequencer_state = Loadable::Ready(state);
            assert_eq!(abort_mode(&repo), expected, "{state:?}");
        }

        repo.merge_commit_message = Loadable::Ready(Some("Merge branch 'topic'".to_string()));
        assert_eq!(abort_mode(&repo), AbortMode::Merge);
    }
}
