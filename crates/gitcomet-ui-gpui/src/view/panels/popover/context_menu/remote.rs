use super::*;
use crate::localization::Language;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    name: &str,
    language: Language,
) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let shown_in_history = !repo.is_some_and(|repo| repo.history_hidden_remotes().contains(name));
    let remote_url = repo
        .and_then(|repo| repo.remotes.ready())
        .and_then(|remotes| remotes.iter().find(|remote| remote.name == name))
        .and_then(|remote| remote.url.as_deref());
    let web_url = remote_url.and_then(crate::view::permalink::remote_web_url);

    let mut items = vec![ContextMenuItem::Header(
        language.text("Remote", "遠端").into(),
    )];
    items.push(ContextMenuItem::Label(name.to_owned().into()));
    items.push(ContextMenuItem::Separator);
    for (label, icon, collapsed) in [
        (
            language.text("Expand all", "全部展開"),
            "icons/arrow_down_to_line.svg",
            false,
        ),
        (
            language.text("Collapse all", "全部收合"),
            "icons/arrow_up_to_line.svg",
            true,
        ),
    ] {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some(icon.into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::SetBranchGroupCollapsedRecursive {
                section: BranchSection::Remote,
                remote: Some(name.to_owned()),
                path: String::new(),
                collapsed,
            }),
        });
    }
    items.push(ContextMenuItem::Separator);
    let (history_label, history_icon) = if shown_in_history {
        (
            language.text("Hide from history graph", "在提交圖中隱藏"),
            "icons/eye_off.svg",
        )
    } else {
        (
            language.text("Show in history graph", "在提交圖中顯示"),
            "icons/eye.svg",
        )
    };
    items.push(ContextMenuItem::Entry {
        label: history_label.into(),
        icon: Some(history_icon.into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::SetRemoteHistoryVisibility {
            repo_id,
            remote: name.to_owned(),
            hidden: shown_in_history,
        }),
    });
    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: language.text("Fetch", "擷取").into(),
        icon: Some("icons/arrow_down.svg".into()),
        shortcut: Some("F".into()),
        disabled: false,
        action: Box::new(ContextMenuAction::FetchRemote {
            repo_id,
            remote: name.to_owned(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: language
            .text("Prune merged branches", "清理已合併的分支")
            .into(),
        icon: Some("icons/broom.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::PruneMergedBranches { repo_id }),
    });
    items.push(ContextMenuItem::Entry {
        label: language.text("Prune local tags", "清理本機標籤").into(),
        icon: Some("icons/tag.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::PruneLocalTags { repo_id }),
    });
    items.push(ContextMenuItem::Separator);
    // Focusing a remote seeds the walk from every branch it tracks, so the graph
    // shows that remote's work and nothing else.
    items.push(super::focus::entry(
        this,
        repo_id,
        gitcomet_core::domain::HistoryFocus::remote(name),
        language,
    ));
    items.push(ContextMenuItem::Separator);

    let open_in_browser_ix = items.len();
    let open_in_browser_tooltip = match (&web_url, remote_url) {
        (Some(url), _) => url.clone(),
        (None, Some(_)) => language
            .text(
                "This remote's URL doesn't point to a web page",
                "這個遠端的網址未指向網頁",
            )
            .to_owned(),
        (None, None) => language
            .text("This remote has no URL", "這個遠端沒有網址")
            .to_owned(),
    };
    items.push(ContextMenuItem::Entry {
        label: language
            .text("Open in web browser", "在瀏覽器中開啟")
            .into(),
        icon: Some("icons/link.svg".into()),
        shortcut: None,
        disabled: web_url.is_none(),
        action: Box::new(ContextMenuAction::OpenWebUrl {
            url: web_url.unwrap_or_default(),
        }),
    });

    for (label, kind) in [
        (
            language.text("Edit fetch URL…", "編輯擷取網址…"),
            RemoteUrlKind::Fetch,
        ),
        (
            language.text("Edit push URL…", "編輯推送網址…"),
            RemoteUrlKind::Push,
        ),
    ] {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some("icons/pencil.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::remote(
                    repo_id,
                    RemotePopoverKind::EditUrlPrompt {
                        name: name.to_owned(),
                        kind,
                    },
                ),
            }),
        });
    }

    items.push(ContextMenuItem::Separator);
    items.push(ContextMenuItem::Entry {
        label: language.text("Remove remote…", "移除遠端…").into(),
        icon: Some("icons/trash.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::OpenPopover {
            kind: PopoverKind::remote(
                repo_id,
                RemotePopoverKind::RemoveConfirm {
                    name: name.to_owned(),
                },
            ),
        }),
    });

    ContextMenuModel::new(items).with_entry_tooltips(FxHashMap::from_iter([(
        open_in_browser_ix,
        SharedString::from(open_in_browser_tooltip),
    )]))
}
