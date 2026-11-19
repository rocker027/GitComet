use super::*;

/// One row per remote, so the palette's Edit Remote URL can pick which
/// remote's fetch URL to edit. Digits pick directly; the push URL stays in
/// each remote's own menu.
pub(super) fn model(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    remote_picker(
        this,
        repo_id,
        "Edit Remote URL",
        "icons/pencil.svg",
        |name| RemotePopoverKind::EditUrlPrompt {
            name,
            kind: RemoteUrlKind::Fetch,
        },
    )
}

/// The palette's Remove Remote: each row opens the remote's existing
/// removal confirmation, so nothing is removed from the menu itself.
pub(super) fn remove_model(this: &PopoverHost, repo_id: RepoId) -> ContextMenuModel {
    remote_picker(this, repo_id, "Remove Remote", "icons/trash.svg", |name| {
        RemotePopoverKind::RemoveConfirm { name }
    })
}

fn remote_picker(
    this: &PopoverHost,
    repo_id: RepoId,
    title: &'static str,
    icon: &'static str,
    next: impl Fn(String) -> RemotePopoverKind,
) -> ContextMenuModel {
    let remotes = this
        .state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.remotes.ready())
        .cloned()
        .unwrap_or_default();

    let mut items = vec![
        ContextMenuItem::Header(title.into()),
        ContextMenuItem::Separator,
    ];
    // The remotes can change under an open picker.
    if remotes.is_empty() {
        items.push(ContextMenuItem::Label(
            "This repository has no remotes".into(),
        ));
        return ContextMenuModel::new(items);
    }

    let mut debug_selectors = FxHashMap::default();
    for (n, remote) in remotes.iter().enumerate() {
        let ix = items.len();
        debug_selectors.insert(ix, format!("remote_picker_{n}").into());
        let label = match remote.url.as_deref() {
            // Credentials in a URL stay out of the menu, as elsewhere.
            Some(url) => format!(
                "{} — {}",
                remote.name,
                gitcomet_core::text_utils::redact_url_userinfo(url)
            ),
            None => remote.name.clone(),
        };
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some(icon.into()),
            shortcut: (n < 9).then(|| (n + 1).to_string().into()),
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover {
                kind: PopoverKind::remote(repo_id, next(remote.name.clone())),
            }),
        });
    }
    ContextMenuModel::new(items).with_entry_debug_selectors(debug_selectors)
}
