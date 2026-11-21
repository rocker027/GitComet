use super::*;

use crate::kit::components::secondary_shortcut;
use crate::localization::Language;

pub(in super::super) fn model(
    repo_id: RepoId,
    path: &std::path::Path,
    branch: Option<&str>,
    editor_available: bool,
    location_available: bool,
    protected_by_finish: bool,
    language: Language,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header(
        language.text("Worktree", "工作樹").into(),
    )];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: language.text("Open in new tab", "在新分頁開啟").into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenRepo {
            path: path.to_path_buf(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: language
            .text("Open folder location", "開啟所在資料夾")
            .into(),
        icon: Some("icons/folder.svg".into()),
        shortcut: None,
        disabled: !location_available,
        action: Box::new(ContextMenuAction::OpenRepositoryLocation {
            path: path.to_path_buf(),
        }),
    });
    if editor_available {
        items.push(ContextMenuItem::Entry {
            label: language
                .text(
                    crate::menu_labels::OPEN_IN_CODE_EDITOR,
                    "在程式碼編輯器中開啟",
                )
                .into(),
            icon: Some("icons/open_external.svg".into()),
            shortcut: Some(secondary_shortcut("Shift+E").into()),
            disabled: false,
            action: Box::new(ContextMenuAction::OpenInCodeEditor {
                repo_id: None,
                path: path.to_path_buf(),
            }),
        });
    }
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: language.text("Finish into…", "收尾到…").into(),
        icon: Some("icons/swap.svg".into()),
        shortcut: None,
        disabled: branch.is_none() || protected_by_finish,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(
                repo_id,
                WorktreePopoverKind::FinishSetup {
                    preselected: vec![path.to_path_buf()],
                    target: None,
                    mode: None,
                },
            ),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: language.text("Remove…", "移除…").into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: protected_by_finish,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(
                repo_id,
                WorktreePopoverKind::RemoveConfirm {
                    path: path.to_path_buf(),
                    branch: branch.map(ToOwned::to_owned),
                },
            ),
        }),
    });

    ContextMenuModel::new(items)
}

/// The worktrees on one history row, each opening in a tab.
pub(super) fn commit_model(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
    language: Language,
) -> ContextMenuModel {
    let sha = commit_id.as_ref();
    let sha = sha.get(..8).unwrap_or(sha);
    let title = match language {
        Language::English => format!("Worktrees on {sha}"),
        Language::TraditionalChinese => format!("提交 {sha} 的工作樹"),
    };
    let mut model = ContextMenuModel::new(vec![
        ContextMenuItem::Header(title.into()),
        ContextMenuItem::Separator,
    ]);
    let Some(repo) = this.state.repos.iter().find(|repo| repo.id == repo_id) else {
        return model;
    };
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return model;
    };
    let dirty = match &repo.worktree_dirty {
        Loadable::Ready(dirty) => dirty.as_slice(),
        _ => &[],
    };
    let chips = crate::view::caches::history_commit_worktree_chips(
        &crate::view::caches::history_worktree_chips_by_head(worktrees, &repo.spec.workdir),
        commit_id,
        dirty,
    );
    for (chip_ix, chip) in chips.into_iter().enumerate() {
        let index = model.items.len();
        model
            .entry_tooltips
            .insert(index, chip.path.display().to_string().into());
        model
            .entry_debug_selectors
            .insert(index, format!("history_worktree_menu_{chip_ix}").into());
        model.items.push(ContextMenuItem::Entry {
            label: chip.label,
            icon: Some("icons/git_worktree.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenRepo { path: chip.path }),
        });
    }
    model
}

impl PopoverHost {
    pub(super) fn history_worktree_menu_is_stale(&self) -> bool {
        use crate::view::panes::history::{
            HISTORY_WORKTREE_ROW_MENU_PREFIX, history_worktree_menu_invoker,
            history_worktree_menu_target,
        };

        let Some(invoker) = self
            .active_invoker
            .as_ref()
            .filter(|invoker| invoker.starts_with(HISTORY_WORKTREE_ROW_MENU_PREFIX))
        else {
            return false;
        };
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu { path, branch }),
        }) = self.popover.as_ref()
        else {
            return false;
        };
        self.store.snapshot().is_none_or(|state| {
            history_worktree_menu_target(&state, *repo_id, path).is_none_or(
                |(lifetime, current)| {
                    current != branch.as_deref()
                        || *invoker != history_worktree_menu_invoker(*repo_id, lifetime, path)
                },
            )
        })
    }

    /// Snapshot availability once per menu opening, away from the UI thread.
    /// A stat on an offline network share may never return; the menu remains
    /// usable with the location action disabled while the check is pending.
    pub(in super::super) fn prepare_worktree_menu_location(
        &mut self,
        kind: Option<&PopoverKind>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.worktree_menu_location_task = None;
        self.worktree_menu_location = None;
        let Some(PopoverKind::Repo {
            kind: RepoPopoverKind::Worktree(WorktreePopoverKind::Menu { path, .. }),
            ..
        }) = kind
        else {
            return;
        };
        let path = path.clone();
        self.worktree_menu_location = Some((path.clone(), false));
        let check_path = path.clone();
        let check = cx
            .background_executor()
            .spawn(async move { check_path.is_dir() });
        self.worktree_menu_location_task = Some(cx.spawn(async move |host, cx| {
            let available = check.await;
            let _ = host.update(cx, |host, cx| {
                if let Some((current_path, current_available)) = &mut host.worktree_menu_location
                    && *current_path == path
                {
                    *current_available = available;
                    cx.notify();
                }
            });
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_location_uses_cached_availability_without_checking_disk() {
        let path = std::path::Path::new("/unavailable/checkout");
        for available in [false, true] {
            let menu = model(
                RepoId(1),
                path,
                None,
                false,
                available,
                false,
                Language::English,
            );
            let disabled = menu.items.iter().find_map(|item| match item {
                ContextMenuItem::Entry {
                    action, disabled, ..
                } if matches!(
                    action.as_ref(),
                    ContextMenuAction::OpenRepositoryLocation { .. }
                ) =>
                {
                    Some(*disabled)
                }
                _ => None,
            });
            assert_eq!(disabled, Some(!available));
        }
    }

    #[test]
    fn model_includes_open_in_new_tab() {
        let repo_id = RepoId(1);
        let path = std::path::PathBuf::from("/tmp/worktree");
        let model = model(repo_id, &path, None, false, true, false, Language::English);

        let open_action = model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "Open in new tab" =>
                {
                    Some((**action).clone())
                }
                _ => None,
            })
            .expect("expected Open in new tab entry");

        assert!(matches!(
            open_action,
            ContextMenuAction::OpenRepo { path: open_path } if open_path == path
        ));
    }

    #[test]
    fn model_routes_remove_through_branch_aware_confirm_when_branch_is_provided() {
        let repo_id = RepoId(1);
        let path = std::path::PathBuf::from("/tmp/worktree");
        for (language, remove_label) in [
            (Language::English, "Remove…"),
            (Language::TraditionalChinese, "移除…"),
        ] {
            let menu = model(
                repo_id,
                &path,
                Some("feature/工作樹"),
                false,
                true,
                false,
                language,
            );
            let remove_action = menu
                .items
                .iter()
                .find_map(|item| match item {
                    ContextMenuItem::Entry { label, action, .. }
                        if label.as_ref() == remove_label =>
                    {
                        Some(action.as_ref())
                    }
                    _ => None,
                })
                .expect("expected Remove entry");

            assert!(matches!(
                remove_action,
                ContextMenuAction::OpenPopover {
                    kind: PopoverKind::Repo {
                        repo_id: rid,
                        kind: RepoPopoverKind::Worktree(WorktreePopoverKind::RemoveConfirm {
                            path: remove_path,
                            branch: Some(branch),
                        }),
                    },
                } if *rid == repo_id && *remove_path == path && branch == "feature/工作樹"
            ));
        }
    }

    #[test]
    fn window_editor_override_exposes_the_worktree_action() {
        let path = std::path::Path::new("/tmp/worktree");
        let has_editor = |available| {
            model(
                RepoId(1),
                path,
                None,
                available,
                true,
                false,
                Language::English,
            )
            .items
            .iter()
            .any(|item| {
                matches!(item, ContextMenuItem::Entry { action, .. }
                if matches!(**action, ContextMenuAction::OpenInCodeEditor { .. }))
            })
        };
        assert!(!has_editor(false));
        assert!(has_editor(true));
    }
}
