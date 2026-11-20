use super::*;
use crate::localization::Language;

use crate::kit::components::secondary_shortcut;

/// What a commit-diff file row belongs to: one commit's details, or the
/// comparison view (`to: None` compares against the working tree).
#[derive(Clone, Copy)]
pub(super) enum FileMenuSource<'a> {
    Commit(&'a CommitId),
    Range {
        from: &'a CommitId,
        to: Option<&'a CommitId>,
    },
}

impl<'a> FileMenuSource<'a> {
    fn diff_target(self, path: &std::path::Path) -> DiffTarget {
        let path = path.to_path_buf();
        match self {
            Self::Commit(commit_id) => DiffTarget::commit(commit_id.clone(), path),
            Self::Range { from, to } => {
                DiffTarget::commit_range(from.clone(), to.cloned(), Some(path))
            }
        }
    }

    /// The revision the file content is at, for a permalink. A comparison to
    /// the working tree has none.
    fn tip(self) -> Option<&'a CommitId> {
        match self {
            Self::Commit(commit_id) => Some(commit_id),
            Self::Range { to, .. } => to,
        }
    }

    fn list(self) -> crate::view::rows::FileListId {
        match self {
            Self::Commit(_) => crate::view::rows::FileListId::CommitFiles,
            Self::Range { .. } => crate::view::rows::FileListId::RangeFiles,
        }
    }

    /// Where "Apply change" takes this file's change from. A comparison to
    /// the working tree has no change to apply: it is already in the checkout.
    pub(super) fn apply_source(self) -> Option<gitcomet_core::domain::ApplyChangeSource> {
        use gitcomet_core::domain::ApplyChangeSource;
        match self {
            Self::Commit(commit_id) => Some(ApplyChangeSource::Commit(commit_id.clone())),
            Self::Range { from, to } => to.map(|to| ApplyChangeSource::Range {
                from: from.clone(),
                to: to.clone(),
            }),
        }
    }
}

/// "Apply change" for files of a commit or comparison. Shares cherry-pick's
/// icon and is disabled while history is being rewritten, like cherry-pick.
pub(super) fn apply_change_entry(
    this: &PopoverHost,
    repo_id: RepoId,
    target: gitcomet_core::domain::ApplyChangeTarget,
    language: Language,
) -> ContextMenuItem {
    let busy = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .is_none_or(|repo| {
            repo.history_rewrite_busy()
                || this.state.worktree_finish_blocks_workdir_mutation(repo.id)
        });
    let label = match (language, target.paths.len()) {
        (_, 1) => language.text("Apply change", "套用變更").into(),
        (Language::English, count) => format!("Apply changes ({count})").into(),
        (Language::TraditionalChinese, count) => format!("套用變更（{count}）").into(),
    };
    ContextMenuItem::Entry {
        label,
        icon: Some("icons/arrow_up.svg".into()),
        shortcut: Some("A".into()),
        disabled: busy,
        action: Box::new(ContextMenuAction::ApplyFileChange { repo_id, target }),
    }
}

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    source: FileMenuSource<'_>,
    path: &std::path::Path,
    cx: &gpui::Context<PopoverHost>,
    language: Language,
) -> ContextMenuModel {
    let is_submodule = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| {
            let files = match source {
                FileMenuSource::Commit(commit_id) => match &repo.history_state.commit_details {
                    Loadable::Ready(details) if details.id == *commit_id => &details.files,
                    _ => return None,
                },
                FileMenuSource::Range { .. } => match &repo.history_state.range_files {
                    Loadable::Ready(files) => files.as_ref(),
                    _ => return None,
                },
            };
            files
                .iter()
                .find(|file| file.path == path)
                .map(|file| file.is_submodule)
        })
        .unwrap_or(false);

    let mut items = vec![ContextMenuItem::Header(
        path.file_name()
            .and_then(|p| p.to_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{path:?}"))
            .into(),
    )];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    if is_submodule {
        let submodule_state = super::submodule::menu_state(this, repo_id, path);
        if let Some(status_label) = super::submodule::status_label(submodule_state.status) {
            items.push(ContextMenuItem::Label(status_label.into()));
        }
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Entry {
            label: language
                .text("Open submodule summary", "開啟子模組摘要")
                .into(),
            icon: Some("icons/box.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SelectDiff {
                repo_id,
                target: source.diff_target(path),
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: language.text("Open submodule", "開啟子模組").into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: None,
            disabled: !submodule_state.can_open,
            action: Box::new(ContextMenuAction::OpenRepo {
                path: submodule_state.open_path.clone().unwrap_or_default(),
            }),
        });
        if this.external_editor_available() {
            items.push(ContextMenuItem::Entry {
                label: language
                    .text("Open in code editor", "在程式碼編輯器中開啟")
                    .into(),
                icon: Some("icons/open_external.svg".into()),
                shortcut: Some(secondary_shortcut("E").into()),
                disabled: !submodule_state.can_open,
                action: Box::new(ContextMenuAction::OpenInCodeEditor {
                    repo_id: Some(repo_id),
                    path: path.to_path_buf(),
                }),
            });
        }
        if submodule_state.show_load {
            items.push(ContextMenuItem::Entry {
                label: language.text("Load submodule", "載入子模組").into(),
                icon: Some("icons/plus.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::LoadSubmodule {
                    repo_id,
                    path: path.to_path_buf(),
                }),
            });
        }
        push_copy_path_entries(&mut items, this, repo_id, path, Some("C".into()), language);
        return ContextMenuModel::new(items);
    }

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: language.text("Open file", "開啟檔案").into(),
        icon: Some("icons/file.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenFile {
            repo_id,
            path: path.to_path_buf(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: language.text("Edit file", "編輯檔案").into(),
        icon: Some("icons/pencil.svg".into()),
        shortcut: None,
        disabled: crate::view::should_bypass_text_file_preview_for_path(path),
        action: Box::new(ContextMenuAction::EditFile {
            repo_id,
            path: path.to_path_buf(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: language.text("Open file location", "開啟檔案位置").into(),
        icon: Some("icons/folder.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenFileLocation {
            repo_id,
            path: path.to_path_buf(),
        }),
    });
    if this.external_editor_available() {
        items.push(ContextMenuItem::Entry {
            label: language
                .text("Open in code editor", "在程式碼編輯器中開啟")
                .into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: Some(secondary_shortcut("E").into()),
            disabled: false,
            action: Box::new(ContextMenuAction::OpenInCodeEditor {
                repo_id: Some(repo_id),
                path: path.to_path_buf(),
            }),
        });
    }
    items.push(ContextMenuItem::Entry {
        label: language.text("File history", "檔案歷史").into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: Some("H".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::OpenFileHistory {
            repo_id,
            path: path.to_path_buf(),
        }),
    });
    if let Some(apply_source) = source.apply_source() {
        // A right-click inside a multi-selection applies the whole selection.
        let paths =
            this.details_pane
                .read(cx)
                .commit_list_paths_for_action(repo_id, source.list(), path);
        items.push(ContextMenuItem::Separator);
        items.push(apply_change_entry(
            this,
            repo_id,
            gitcomet_core::domain::ApplyChangeTarget {
                source: apply_source,
                paths,
            },
            language,
        ));
        items.push(ContextMenuItem::Separator);
    }
    if let Some(permalink) = source.tip().and_then(|tip| {
        let repo = this.state.repos.iter().find(|repo| repo.id == repo_id)?;
        match &repo.remotes {
            Loadable::Ready(remotes) => crate::view::permalink::file_permalink(
                remotes,
                tip.as_ref(),
                &path.display().to_string(),
            ),
            _ => None,
        }
    }) {
        items.push(ContextMenuItem::Entry {
            label: language
                .text("Copy file permalink", "複製檔案的永久連結")
                .into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText { text: permalink }),
        });
    }
    push_copy_path_entries(&mut items, this, repo_id, path, Some("C".into()), language);

    ContextMenuModel::new(items)
}
