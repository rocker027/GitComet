use super::*;
use crate::localization::Language;
use crate::view::rows::FileListId;
use gitcomet_core::domain::{ApplyChangeSource, ApplyChangeTarget};

/// The folder row a menu was opened on.
pub(super) struct FolderMenu<'a> {
    pub(super) repo_id: RepoId,
    pub(super) list: FileListId,
    pub(super) key: &'a Arc<std::path::Path>,
    pub(super) chain: &'a Arc<[Arc<std::path::Path>]>,
    pub(super) collapsed: bool,
    pub(super) apply_source: Option<&'a ApplyChangeSource>,
}

/// Context menu for a folder row in a file list's tree view: opening and
/// closing it, its files' actions applied to the whole folder, and its path.
pub(super) fn model(
    this: &PopoverHost,
    folder: FolderMenu<'_>,
    cx: &gpui::Context<PopoverHost>,
    language: Language,
) -> ContextMenuModel {
    let FolderMenu {
        repo_id,
        list,
        key,
        chain,
        collapsed,
        apply_source,
    } = folder;
    let path: &std::path::Path = key;
    let files = this
        .details_pane
        .read(cx)
        .file_list_folder_files(repo_id, list, path);
    let collapse_action = |collapsed: bool, recursive: bool| {
        Box::new(ContextMenuAction::SetFileListFolderCollapsed {
            repo_id,
            list,
            key: Arc::clone(key),
            chain: Arc::clone(chain),
            collapsed,
            recursive,
        })
    };

    let mut items = vec![ContextMenuItem::Header(
        path.file_name()
            .and_then(|name| name.to_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| path.display().to_string())
            .into(),
    )];
    items.push(ContextMenuItem::Label(
        components::ContextMenuText::path_single_line(path.display().to_string()),
    ));
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: if collapsed {
            language.text("Expand", "展開")
        } else {
            language.text("Collapse", "收合")
        }
        .into(),
        icon: Some(
            if collapsed {
                "icons/chevron_right.svg"
            } else {
                "icons/chevron_down.svg"
            }
            .into(),
        ),
        shortcut: None,
        disabled: false,
        action: collapse_action(!collapsed, false),
    });
    items.push(ContextMenuItem::Entry {
        label: language
            .text("Expand all under here", "展開下方所有資料夾")
            .into(),
        icon: Some("icons/arrow_down_to_line.svg".into()),
        shortcut: None,
        disabled: false,
        action: collapse_action(false, true),
    });
    items.push(ContextMenuItem::Entry {
        label: language
            .text("Collapse all under here", "收合下方所有資料夾")
            .into(),
        icon: Some("icons/arrow_up_to_line.svg".into()),
        shortcut: None,
        disabled: false,
        action: collapse_action(true, true),
    });

    match list {
        FileListId::Status(section) => {
            let count = files.len();
            let staged = section.diff_area() == DiffArea::Staged;
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Entry {
                label: match (language, staged) {
                    (Language::English, true) => format!("Unstage folder ({count})"),
                    (Language::English, false) => format!("Stage folder ({count})"),
                    (Language::TraditionalChinese, true) => {
                        format!("取消暫存資料夾變更（{count}）")
                    }
                    (Language::TraditionalChinese, false) => {
                        format!("暫存資料夾變更（{count}）")
                    }
                }
                .into(),
                icon: Some(
                    if staged {
                        "icons/minus.svg"
                    } else {
                        "icons/plus.svg"
                    }
                    .into(),
                ),
                shortcut: Some("S".into()),
                disabled: count == 0,
                action: Box::new(ContextMenuAction::StageStatusFolder {
                    repo_id,
                    section,
                    key: Arc::clone(key),
                }),
            });
            items.push(ContextMenuItem::Entry {
                label: language.text("Discard changes…", "捨棄變更…").into(),
                icon: Some("icons/refresh.svg".into()),
                shortcut: None,
                disabled: count == 0,
                action: Box::new(ContextMenuAction::OpenPopover {
                    kind: PopoverKind::DiscardFolderChangesConfirm {
                        repo_id,
                        section,
                        folder: Arc::clone(key),
                    },
                }),
            });
            items.push(ContextMenuItem::Separator);
            items.push(ContextMenuItem::Entry {
                label: language
                    .text("Open folder location", "開啟資料夾位置")
                    .into(),
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
                    shortcut: None,
                    disabled: false,
                    action: Box::new(ContextMenuAction::OpenInCodeEditor {
                        repo_id: Some(repo_id),
                        path: path.to_path_buf(),
                    }),
                });
            }
        }
        FileListId::CommitFiles | FileListId::RangeFiles => {
            // Submodules are left out of `files`; a folder of nothing else
            // has no file change to apply.
            if let Some(source) = apply_source
                && !files.is_empty()
            {
                items.push(ContextMenuItem::Separator);
                items.push(super::commit_file::apply_change_entry(
                    this,
                    repo_id,
                    ApplyChangeTarget {
                        source: source.clone(),
                        paths: files,
                    },
                    language,
                ));
            }
        }
        FileListId::WorktreeFiles => {}
    }

    items.push(ContextMenuItem::Separator);
    if list == FileListId::WorktreeFiles {
        // Its paths are relative to the other worktree, which an absolute
        // path resolved against this repository would get wrong.
        items.push(ContextMenuItem::Entry {
            label: language.text("Copy relative path", "複製相對路徑").into(),
            icon: Some("icons/copy.svg".into()),
            shortcut: Some("C".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::CopyText {
                text: path.display().to_string(),
            }),
        });
    } else {
        push_copy_path_entries(&mut items, this, repo_id, path, Some("C".into()), language);
    }

    ContextMenuModel::new(items)
}
