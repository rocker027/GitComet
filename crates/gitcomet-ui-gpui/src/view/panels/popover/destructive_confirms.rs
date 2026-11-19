//! Confirmations for deletions reached from menus and the command palette:
//! nothing is deleted before the user confirms here.
use super::*;

pub(super) fn delete_tag_panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    name: String,
    remote: Option<String>,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let (title, text, command) = match &remote {
        None => (
            "Delete tag?",
            "This removes the tag from this repository. A copy pushed to a remote stays there."
                .to_string(),
            format!("git tag -d {name}"),
        ),
        Some(remote) => (
            "Delete tag from remote?",
            format!("This removes the tag from {remote} for everyone who fetches from it."),
            format!("git push --delete {remote} refs/tags/{name}"),
        ),
    };
    ConfirmDialog::new(title, DIALOG_420_WIDTH)
        .mono_value(theme, name.clone())
        .text(theme, text)
        .command(theme, command)
        .render(
            theme,
            cancel_button("delete_tag_confirm_cancel", "delete_tag_cancel_hint", theme).on_click(
                theme,
                cx,
                |this, _e, _w, cx| this.close_popover(cx),
            ),
            components::Button::new("delete_tag_confirm_go", "Delete")
                .style(components::ButtonStyle::Danger)
                .on_click(theme, cx, move |this, _e, _w, cx| {
                    match remote.clone() {
                        None => this.store.dispatch(Msg::DeleteTag {
                            repo_id,
                            name: name.clone(),
                        }),
                        Some(remote) => this.store.dispatch(Msg::DeleteRemoteTag {
                            repo_id,
                            remote,
                            name: name.clone(),
                        }),
                    }
                    this.close_popover(cx);
                }),
            cx,
        )
}

/// What "Discard all" would restore: every unstaged and untracked path except
/// submodules, whose discard also wipes uncommitted work inside them and is
/// left to their own rows, with how many were left out. `None` until both the
/// status and the submodule list have loaded.
pub(super) fn discard_all_paths(repo: &RepoState) -> Option<(Vec<std::path::PathBuf>, usize)> {
    let entries = repo.worktree_status_entries()?;
    let submodules = repo.submodules.ready()?;
    let is_submodule = |path: &std::path::Path| {
        submodules
            .iter()
            .any(|submodule| submodule.path.as_path() == path)
    };
    let skipped = entries
        .iter()
        .filter(|entry| is_submodule(&entry.path))
        .count();
    let paths = entries
        .iter()
        .filter(|entry| !is_submodule(&entry.path))
        .map(|entry| entry.path.clone())
        .collect();
    Some((paths, skipped))
}

pub(super) fn discard_all_panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    // Like the status panel's Discard, wait for the running local action:
    // a stage still in flight could otherwise turn a new file into one the
    // discard removes.
    let busy = repo.is_some_and(|repo| repo.local_actions_in_flight > 0);
    let listed = repo.and_then(discard_all_paths);
    let (paths, skipped) = listed.clone().unwrap_or_default();
    let count = paths.len();
    let read_error = repo.and_then(|repo| match (&repo.status, &repo.submodules) {
        (Loadable::Error(error), _) => Some(format!("Could not read the changes: {error}")),
        (_, Loadable::Error(error)) => Some(format!("Could not list the submodules: {error}")),
        _ => None,
    });
    let summary = match (&listed, count) {
        (None, _) => read_error.unwrap_or_else(|| "Reading the changes…".to_string()),
        (Some(_), 0) => "There are no unstaged changes to discard.".to_string(),
        (Some(_), 1) => "1 file".to_string(),
        (Some(_), n) => format!("{n} files"),
    };
    let conflict_note = super::discard_changes_confirm::conflict_discard_note(
        &this.state,
        repo_id,
        paths.iter().map(|path| path.as_path()),
    );
    let mut dialog = ConfirmDialog::new("Discard all unstaged changes?", DIALOG_420_WIDTH)
        .mono_value(theme, summary)
        .text(
            theme,
            "This restores every unstaged file and removes untracked files. Staged changes stay, except conflicted files, which are resolved and staged. It cannot be undone.",
        );
    if let Some(note) = conflict_note {
        dialog = dialog.note(theme, note);
    }
    if skipped > 0 {
        let noun = if skipped == 1 {
            "submodule"
        } else {
            "submodules"
        };
        dialog = dialog.note(
            theme,
            format!("{skipped} changed {noun} left alone: discard them from their own rows."),
        );
    }
    dialog.render(
        theme,
        cancel_button(
            "discard_all_confirm_cancel",
            "discard_all_cancel_hint",
            theme,
        )
        .on_click(theme, cx, |this, _e, _w, cx| this.close_popover(cx)),
        components::Button::new("discard_all_confirm_go", "Discard")
            .style(components::ButtonStyle::Danger)
            .disabled(count == 0 || busy)
            .on_click(theme, cx, move |this, _e, _w, cx| {
                if !paths.is_empty() {
                    this.dispatch_open_file_change(
                        Msg::DiscardWorktreeChangesPaths {
                            repo_id,
                            paths: paths.clone(),
                        },
                        cx,
                    );
                }
                this.close_popover(cx);
            }),
        cx,
    )
}
