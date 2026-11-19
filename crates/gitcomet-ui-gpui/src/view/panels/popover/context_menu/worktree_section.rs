use super::*;
use crate::localization::Language;

pub(super) fn model(repo_id: RepoId, language: Language) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header(
        language.text("Worktrees", "工作樹").into(),
    )];
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: language.text("Add worktree…", "新增工作樹…").into(),
        icon: Some("icons/plus.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: language.text("Refresh worktrees", "重新整理工作樹").into(),
        icon: Some("icons/refresh.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::LoadWorktrees { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: language.text("Open worktree…", "開啟工作樹…").into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(repo_id, WorktreePopoverKind::OpenPicker),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: language.text("Remove worktree…", "移除工作樹…").into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::worktree(repo_id, WorktreePopoverKind::RemovePicker),
        }),
    });

    ContextMenuModel::new(items)
}
