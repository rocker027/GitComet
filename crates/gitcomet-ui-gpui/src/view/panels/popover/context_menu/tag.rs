use super::*;
use crate::localization::Language;
use rustc_hash::FxHashSet;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
    language: Language,
) -> ContextMenuModel {
    let sha = commit_id.as_ref().to_string();
    let short: SharedString = sha.get(0..8).unwrap_or(&sha).to_string().into();
    let title = match language {
        Language::English => format!("Tags on {short}"),
        Language::TraditionalChinese => format!("{short} 的標籤"),
    }
    .into();

    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let tags = match repo.map(|r| &r.tags) {
        Some(Loadable::Ready(tags)) => Some(tags.as_slice()),
        Some(Loadable::Error(err)) => {
            return ContextMenuModel::new(vec![
                ContextMenuItem::Header(title),
                ContextMenuItem::Separator,
                ContextMenuItem::Label(err.clone().into()),
            ]);
        }
        Some(Loadable::Loading) | Some(Loadable::NotLoaded) => {
            return ContextMenuModel::new(vec![
                ContextMenuItem::Header(title),
                ContextMenuItem::Separator,
                ContextMenuItem::Label(language.text("Loading tags…", "正在載入標籤…").into()),
            ]);
        }
        None => None,
    }
    .unwrap_or(&[]);
    let (remote_names, remote_tags) = remote_tag_context(repo);

    let mut tag_names = tags
        .iter()
        .filter(|t| t.target == *commit_id)
        .map(|t| t.name.clone())
        .collect::<Vec<_>>();
    tag_names.sort_unstable();

    let compare_label = tag_names
        .first()
        .cloned()
        .unwrap_or_else(|| short.to_string());
    let comparison_mark = comparison_mark_pair(repo);

    tag_names_model(
        repo_id,
        title,
        tag_names,
        remote_names,
        remote_tags,
        commit_id,
        compare_label,
        comparison_mark,
        language,
    )
}

pub(super) fn model_for_tag(
    this: &PopoverHost,
    repo_id: RepoId,
    commit_id: &CommitId,
    name: &str,
    language: Language,
) -> ContextMenuModel {
    let sha = commit_id.as_ref().to_string();
    let short = sha.get(0..8).unwrap_or(&sha);
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let (remote_names, remote_tags) = remote_tag_context(repo);
    let comparison_mark = comparison_mark_pair(repo);
    tag_names_model(
        repo_id,
        match language {
            Language::English => format!("Tag {name} on {short}"),
            Language::TraditionalChinese => format!("{short} 的標籤 {name}"),
        }
        .into(),
        vec![name.to_owned()],
        remote_names,
        remote_tags,
        commit_id,
        name.to_owned(),
        comparison_mark,
        language,
    )
}

fn comparison_mark_pair(repo: Option<&RepoState>) -> Option<(CommitId, String)> {
    repo.and_then(|r| {
        r.navigation
            .comparison_mark
            .as_ref()
            .map(|mark| (mark.commit_id.clone(), mark.label.clone()))
    })
}

fn remote_tag_context(repo: Option<&RepoState>) -> (Vec<String>, FxHashSet<(&str, &str)>) {
    let mut remote_names = repo
        .and_then(|r| match &r.remotes {
            Loadable::Ready(remotes) => Some(
                remotes
                    .iter()
                    .map(|remote| remote.name.clone())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    remote_names.sort_unstable();
    remote_names.dedup();
    let remote_tags: FxHashSet<(&str, &str)> = repo
        .and_then(|r| match &r.remote_tags {
            Loadable::Ready(tags) => Some(
                tags.iter()
                    .map(|tag| (tag.remote.as_str(), tag.name.as_str()))
                    .collect::<FxHashSet<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_default();

    (remote_names, remote_tags)
}

#[allow(clippy::too_many_arguments)]
fn tag_names_model(
    repo_id: RepoId,
    title: components::ContextMenuText,
    tag_names: Vec<String>,
    remote_names: Vec<String>,
    remote_tags: FxHashSet<(&str, &str)>,
    commit_id: &CommitId,
    compare_label: String,
    comparison_mark: Option<(CommitId, String)>,
    language: Language,
) -> ContextMenuModel {
    let mut items = vec![ContextMenuItem::Header(title)];
    if tag_names.is_empty() {
        items.push(ContextMenuItem::Label(
            language.text("No tags", "沒有標籤").into(),
        ));
        return ContextMenuModel::new(items);
    }

    items.push(ContextMenuItem::Separator);
    // Comparison: mark this tag's commit, or compare it against a mark.
    items.push(ContextMenuItem::Entry {
        label: match language {
            Language::English => format!("Mark {compare_label} for comparison"),
            Language::TraditionalChinese => format!("標記 {compare_label} 以供比較"),
        }
        .into(),
        icon: Some("icons/tag.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::MarkForComparison {
            repo_id,
            commit_id: commit_id.clone(),
            label: compare_label.clone(),
        }),
    });
    items.push(ContextMenuItem::Entry {
        label: language
            .text("Compare with working tree", "與工作樹比較")
            .into(),
        icon: Some("icons/open_external.svg".into()),
        shortcut: None,
        disabled: false,
        action: Box::new(ContextMenuAction::CompareWithWorkingTree {
            repo_id,
            commit_id: commit_id.clone(),
            label: compare_label.clone(),
        }),
    });
    items.extend(super::marked_comparison_entries(
        repo_id,
        commit_id,
        &compare_label,
        comparison_mark
            .as_ref()
            .map(|(mark_commit, mark_label)| (mark_commit, mark_label.as_str())),
        language,
    ));
    items.push(ContextMenuItem::Separator);
    for (tag_ix, name) in tag_names.into_iter().enumerate() {
        if tag_ix > 0 {
            items.push(ContextMenuItem::Separator);
        }
        items.push(ContextMenuItem::Entry {
            label: match language {
                Language::English => format!("Delete tag {name}"),
                Language::TraditionalChinese => format!("刪除標籤 {name}"),
            }
            .into(),
            icon: Some("icons/trash.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::DeleteTag {
                repo_id,
                name: name.clone(),
            }),
        });

        for remote in &remote_names {
            items.push(ContextMenuItem::Entry {
                label: match language {
                    Language::English => format!("Push tag {name} to {remote}"),
                    Language::TraditionalChinese => format!("推送標籤 {name} 至 {remote}"),
                }
                .into(),
                icon: Some("icons/arrow_up.svg".into()),
                shortcut: None,
                disabled: false,
                action: Box::new(ContextMenuAction::PushTag {
                    repo_id,
                    remote: remote.clone(),
                    name: name.clone(),
                }),
            });
            if remote_tags.contains(&(remote.as_str(), name.as_str())) {
                items.push(ContextMenuItem::Entry {
                    label: match language {
                        Language::English => format!("Delete tag {name} from {remote}"),
                        Language::TraditionalChinese => format!("刪除 {remote} 上的標籤 {name}"),
                    }
                    .into(),
                    icon: Some("icons/trash.svg".into()),
                    shortcut: None,
                    disabled: false,
                    action: Box::new(ContextMenuAction::DeleteRemoteTag {
                        repo_id,
                        remote: remote.clone(),
                        name: name.clone(),
                    }),
                });
            }
        }
    }

    ContextMenuModel::new(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traditional_chinese_tag_menu_keeps_remote_and_tag_targets() {
        let repo_id = RepoId(1);
        let tag = "release/候選版";
        let remote = "review/正式";
        let commit = CommitId("0123456789abcdef".into());
        let model = tag_names_model(
            repo_id,
            "標籤".into(),
            vec![tag.into()],
            vec![remote.into()],
            FxHashSet::from_iter([(remote, tag)]),
            &commit,
            tag.into(),
            None,
            Language::TraditionalChinese,
        );
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "推送標籤 release/候選版 至 review/正式"
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::PushTag {
                                repo_id: action_repo,
                                remote: action_remote,
                                name,
                            } if *action_repo == repo_id && action_remote == remote && name == tag
                        )
            )
        }));
        assert!(model.items.iter().any(|item| {
            matches!(
                item,
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == "刪除 review/正式 上的標籤 release/候選版"
                        && matches!(
                            action.as_ref(),
                            ContextMenuAction::DeleteRemoteTag {
                                repo_id: action_repo,
                                remote: action_remote,
                                name,
                            } if *action_repo == repo_id && action_remote == remote && name == tag
                        )
            )
        }));
    }
}
