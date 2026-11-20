use super::*;

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    target: String,
    mode: ResetMode,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let mode_label = match mode {
        ResetMode::Soft => "--soft",
        ResetMode::Mixed => "--mixed",
        ResetMode::Hard => "--hard",
    };

    let finish_busy = this.state.worktree_finish_blocks_workdir_mutation(repo_id);

    ConfirmDialog::new("Reset", DIALOG_380_WIDTH)
        .text(theme, format!("{mode_label} → {target}"))
        .note(
            theme,
            if finish_busy {
                "This worktree is currently protected by an active Worktree Finish run."
            } else {
                match mode {
                    ResetMode::Hard => "Hard reset updates index + working tree (destructive).",
                    ResetMode::Mixed => "Mixed reset updates index only.",
                    ResetMode::Soft => "Soft reset moves HEAD only.",
                }
            },
        )
        .render(
            theme,
            dialog_cancel_button("reset_cancel", "reset_cancel_hint", theme, cx),
            components::Button::new("reset_go", "Reset")
                .style(components::ButtonStyle::Filled)
                .disabled(finish_busy)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    if this.state.worktree_finish_blocks_workdir_mutation(repo_id) {
                        cx.notify();
                        return;
                    }
                    this.store.dispatch(Msg::Reset {
                        repo_id,
                        target: target.clone(),
                        mode,
                    });
                    this.close_popover(cx);
                }),
            cx,
        )
}
