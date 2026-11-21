use super::*;
use crate::localization::Language;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    section: BranchSection,
    language: Language,
) -> ContextMenuModel {
    let pins = this.pinned_branch_count(repo_id, section);
    let prune_disabled = this.state.has_active_worktree_finish_run(repo_id);
    model_for_section_with_finish(repo_id, section, pins, prune_disabled, language)
}

#[cfg(test)]
fn model_for_section(
    repo_id: RepoId,
    section: BranchSection,
    pins: usize,
    language: Language,
) -> ContextMenuModel {
    model_for_section_with_finish(repo_id, section, pins, false, language)
}

fn model_for_section_with_finish(
    repo_id: RepoId,
    section: BranchSection,
    pins: usize,
    prune_disabled: bool,
    language: Language,
) -> ContextMenuModel {
    let header: SharedString = match section {
        BranchSection::Local => language.text("Local", "本機分支").into(),
        BranchSection::Remote => language.text("Remote", "遠端分支").into(),
    };
    let mut items = vec![ContextMenuItem::Header(header.into())];
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: language.text("Switch branch", "切換分支").into(),
        icon: Some("icons/git_branch.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::BranchPicker {
                purpose: BranchPickerPurpose::Checkout,
            },
        }),
    });

    if section == BranchSection::Remote {
        items.push(ContextMenuItem::Entry {
            label: language.text("Add remote…", "新增遠端…").into(),
            icon: Some("icons/plus.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::remote(repo_id, RemotePopoverKind::AddPrompt),
            }),
        });
        items.push(ContextMenuItem::Entry {
            label: language.text("Fetch all", "擷取全部").into(),
            icon: Some("icons/arrow_down.svg".into()),
            shortcut: Some("F".into()),
            disabled: false,
            action: Box::new(ContextMenuAction::FetchAll { repo_id }),
        });
        items.push(ContextMenuItem::Entry {
            label: language
                .text("Prune merged branches", "清除已合併的分支")
                .into(),
            icon: Some("icons/broom.svg".into()),
            shortcut: None,
            disabled: prune_disabled,
            action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
        });
        items.push(ContextMenuItem::Entry {
            label: language.text("Prune local tags", "清理本機標籤").into(),
            icon: Some("icons/tag.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
        });
    }

    items.push(ContextMenuItem::Separator);
    let (english_section, chinese_section) = match section {
        BranchSection::Local => ("local", "本機分支"),
        BranchSection::Remote => ("remote", "遠端分支"),
    };
    items.push(ContextMenuItem::Entry {
        label: match language {
            Language::English => format!("Unpin all {english_section} ({pins})"),
            Language::TraditionalChinese => format!("取消釘選所有{chinese_section}（{pins}）"),
        }
        .into(),
        icon: Some("icons/pin.svg".into()),
        shortcut: None,
        disabled: pins == 0,
        action: Box::new(ContextMenuAction::UnpinAllBranches { repo_id, section }),
    });
    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_section_header_omits_remote_specific_actions() {
        let repo_id = RepoId(7);
        let model = model_for_section(repo_id, BranchSection::Remote, 0, Language::English);

        let labels: Vec<&str> = model.entry_labels().into_iter().collect();

        assert!(!labels.contains(&"Edit fetch URL…"));
        assert!(!labels.contains(&"Edit push URL…"));
        assert!(!labels.contains(&"Remove remote…"));
        assert!(!labels.contains(&"Delete remote branch…"));
    }
}
