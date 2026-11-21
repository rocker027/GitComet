//! The menu a repository row in the repository picker opens on right-click.
//!
//! Close kin to [`super::repo_tab`], which is the same repository's menu on its
//! tab. This one adds the two things only the picker can do — pinning a
//! repository to the top of the list, and forgetting a closed one — and offers
//! the rest for a repository that is not open at all.

use super::*;
use crate::localization::Language;

pub(super) fn model(
    host: &PopoverHost,
    entry: &repo_picker::RepoPickerEntry,
    language: Language,
) -> ContextMenuModel {
    if let repo_picker::RepoPickerEntry::Workspace(workspace_id) = entry {
        let Some(workspace) = host
            .cached_workspaces
            .iter()
            .find(|workspace| workspace.id == *workspace_id)
        else {
            return ContextMenuModel::new(Vec::new());
        };
        let workspace_id = *workspace_id;
        return ContextMenuModel::new(vec![
            ContextMenuItem::Header(workspace.display_name().into()),
            ContextMenuItem::Entry {
                label: language.text("Activate", "切換至工作區").into(),
                // Same icon as a repository row's Activate.
                icon: Some("icons/check.svg".into()),
                shortcut: None,
                // This window already is that workspace.
                disabled: host.cached_workspace_id == Some(workspace_id),
                action: Box::new(ContextMenuAction::ActivateWorkspace { workspace_id }),
            },
            ContextMenuItem::Separator,
            ContextMenuItem::Entry {
                label: language.text("Workspace Settings", "工作區設定").into(),
                icon: Some("icons/cog.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::OpenWorkspaceSettings { workspace_id }),
            },
            // Destructive last, as on repository rows; the trash icon makes it red.
            ContextMenuItem::Separator,
            ContextMenuItem::Entry {
                label: language.text("Delete workspace", "刪除工作區").into(),
                icon: Some("icons/trash.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::DeleteWorkspace { workspace_id }),
            },
        ]);
    }

    let workdir = entry.workdir(host);
    let pinned = workdir
        .as_ref()
        .is_some_and(|path| host.cached_pinned_repos.iter().any(|pin| pin == path));

    let mut items = Vec::new();
    if let Some(workdir) = workdir.clone() {
        items.push(if pinned {
            ContextMenuItem::Entry {
                label: language.text("Unpin repository", "取消釘選儲存庫").into(),
                icon: Some("icons/pin.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::UnpinRepository { path: workdir }),
            }
        } else {
            ContextMenuItem::Entry {
                label: language.text("Pin repository", "釘選儲存庫").into(),
                icon: Some("icons/pin.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::PinRepository { path: workdir }),
            }
        });
        items.push(ContextMenuItem::Separator);
    }

    match entry {
        repo_picker::RepoPickerEntry::Workspace(_) => {}
        repo_picker::RepoPickerEntry::Open(repo_id) => items.push(ContextMenuItem::Entry {
            label: language.text("Activate", "切換至儲存庫").into(),
            icon: Some("icons/check.svg".into()),
            shortcut: None,
            // The active repository cannot be activated again — a row menu
            // never offers a no-op.
            disabled: host.state.active_repo == Some(*repo_id),
            action: Box::new(ContextMenuAction::ActivateRepo { repo_id: *repo_id }),
        }),
        // Same wording and icon as the "+" menu's Open repository, which is the
        // other way into this action.
        repo_picker::RepoPickerEntry::Closed(path) => items.push(ContextMenuItem::Entry {
            label: language.text("Open repository", "開啟儲存庫").into(),
            icon: Some("icons/disk.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenRepo { path: path.clone() }),
        }),
    }

    if let Some(workdir) = workdir.clone() {
        items.push(ContextMenuItem::Separator);
        items.push(ContextMenuItem::Entry {
            label: language
                .text("Open repository location", "開啟儲存庫所在位置")
                .into(),
            icon: Some("icons/folder.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenRepositoryLocation {
                path: workdir.clone(),
            }),
        });
        if host.external_editor_available() {
            items.push(ContextMenuItem::Entry {
                label: language
                    .text("Open in code editor", "在程式碼編輯器中開啟")
                    .into(),
                icon: Some("icons/open_external.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::OpenInCodeEditor {
                    repo_id: None,
                    path: workdir.clone(),
                }),
            });
        }
        items.push(ContextMenuItem::Entry {
            // A repository has only its workdir, so there is no relative
            // counterpart to pair this with — but it is named the same way as
            // the file menus' entry so the app has one vocabulary for the
            // action.
            label: language.text("Copy absolute path", "複製絕對路徑").into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText {
                text: workdir.display().to_string(),
            }),
        });
    }

    // A pin is what keeps a closed repository listed, so forgetting it while it
    // is pinned would leave the row exactly where it was — the pinned closed row
    // is the one case with nothing to put here at all.
    let destructive = match entry {
        repo_picker::RepoPickerEntry::Workspace(_) => None,
        repo_picker::RepoPickerEntry::Open(repo_id) => Some(ContextMenuItem::Entry {
            label: language.text("Close repository", "關閉儲存庫").into(),
            icon: Some("icons/repo_tab_close.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::CloseRepo { repo_id: *repo_id }),
        }),
        repo_picker::RepoPickerEntry::Closed(_) if pinned => None,
        repo_picker::RepoPickerEntry::Closed(path) => Some(ContextMenuItem::Entry {
            label: language
                .text("Remove from recently closed", "從最近關閉清單移除")
                .into(),
            icon: Some("icons/repo_tab_close.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::ForgetRecentRepository { path: path.clone() }),
        }),
    };
    if let Some(destructive) = destructive {
        items.push(ContextMenuItem::Separator);
        items.push(destructive);
    }

    ContextMenuModel::new(items)
}
