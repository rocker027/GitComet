use super::*;
use crate::localization::Language;

pub(in super::super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
    path: &std::path::Path,
    language: Language,
) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let is_submodule = repo.is_some_and(|repo| {
        repo.head_gitlink_paths.contains(path)
            || this.bound_diff_state(repo).is_some_and(|diff| {
                diff.submodule_summary
                    .ready()
                    .is_some_and(|summary| summary.path == path)
            })
            || repo
                .submodules
                .ready()
                .is_some_and(|modules| modules.iter().any(|module| module.path == path))
            || repo
                .history_state
                .commit_details
                .ready()
                .is_some_and(|details| {
                    details.id == *commit_id
                        && details
                            .files
                            .iter()
                            .any(|file| file.path == path && file.is_submodule)
                })
    });
    let commit = repo
        .and_then(|repo| repo.history_state.file_history.ready())
        .and_then(|page| page.commits.iter().find(|commit| commit.id == *commit_id));
    let sha = commit_id.as_ref();
    let mut items = vec![ContextMenuItem::Header(
        format!(
            "{} {}",
            language.text("Commit", "提交"),
            sha.get(..8).unwrap_or(sha)
        )
        .into(),
    )];
    if let Some(commit) = commit {
        items.push(ContextMenuItem::Label(
            components::ContextMenuText::new(format!("{} — {}", commit.author, commit.summary))
                .max_lines(4),
        ));
    }
    items.push(ContextMenuItem::Separator);
    let mut entry = |label: &str, icon: &'static str, disabled: bool, action: ContextMenuAction| {
        items.push(ContextMenuItem::Entry {
            label: label.to_owned().into(),
            icon: Some(icon.into()),
            shortcut: None,
            disabled,
            action: Box::new(action),
        });
    };
    entry(
        language.text("Open file at this commit", "開啟此提交中的檔案"),
        "icons/file.svg",
        false,
        ContextMenuAction::OpenFileAtCommit {
            repo_id,
            commit_id: commit_id.clone(),
            path: path.to_path_buf(),
        },
    );
    entry(
        language.text("Open file at parent", "開啟父提交中的檔案"),
        "icons/file.svg",
        commit.is_none_or(|commit| commit.parent_ids.is_empty()),
        ContextMenuAction::OpenFileAtCommitParent {
            repo_id,
            commit_id: commit_id.clone(),
            path: path.to_path_buf(),
        },
    );
    entry(
        language.text("Show changes to this file", "顯示此檔案的變更"),
        "icons/open_external.svg",
        false,
        ContextMenuAction::ShowFileChangesAtCommit {
            repo_id,
            commit_id: commit_id.clone(),
            path: path.to_path_buf(),
        },
    );
    entry(
        language.text("Reveal in history", "在歷史中定位"),
        "icons/history.svg",
        false,
        ContextMenuAction::RevealHistoryCommit {
            repo_id,
            commit_id: commit_id.clone(),
        },
    );
    entry(
        language.text("Copy SHA", "複製 SHA"),
        "icons/copy.svg",
        false,
        ContextMenuAction::CopyCommitSha {
            commit_id: commit_id.clone(),
        },
    );
    items.push(ContextMenuItem::Separator);
    if !is_submodule {
        items.push(super::commit_file::apply_change_entry(
            this,
            repo_id,
            gitcomet_core::domain::ApplyChangeTarget::commit(commit_id.clone(), path.to_path_buf()),
            language,
        ));
        items.push(ContextMenuItem::Separator);
    }
    let actions = super::commit::action_items(this, repo_id, commit_id, language);
    let offset = items.len();
    items.extend(actions.items);
    ContextMenuModel::new(items).with_entry_tooltips(
        actions
            .entry_tooltips
            .into_iter()
            .map(|(ix, text)| (ix + offset, text))
            .collect(),
    )
}
