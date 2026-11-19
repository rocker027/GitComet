//! The menu a sidebar branch opens when dropped on another branch, as in
//! GitKraken: merge into, rebase onto, or push to the drop target.
//!
//! Every entry runs the flow its branch-menu counterpart runs: merge, squash
//! and push start at once, as "Merge into current" and the toolbar Push do;
//! rebase asks first, and interactive rebase opens the editor. Merges land
//! in the checked-out branch and rebases move it, so combinations that
//! involve neither side being checked out are listed greyed out with the
//! reason instead of switching branches behind the user's back.
use super::*;
use crate::localization::Language;

/// The repository facts the drop menu depends on.
pub(super) struct BranchDropFacts<'a> {
    /// The checked-out local branch, if any.
    pub(super) current: Option<&'a str>,
    /// The checked-out branch's upstream as `(remote, branch)`.
    pub(super) current_upstream: Option<(&'a str, &'a str)>,
    pub(super) merge_allowed: bool,
    pub(super) history_rewrite_busy: bool,
    /// git-annex runs Push itself, without an upstream (as `push_request`).
    pub(super) annex_pushes: bool,
}

/// A branch end of the drop: its menu target and the reference Git
/// actions use (the exact tip for remote branches, as the branch menu does).
pub(super) struct BranchDropEnd<'a> {
    pub(super) target: &'a BranchMenuTarget,
    pub(super) reference: String,
}

fn entry(
    label: String,
    icon: &'static str,
    disabled: bool,
    action: ContextMenuAction,
) -> ContextMenuItem {
    ContextMenuItem::Entry {
        label: label.into(),
        icon: Some(icon.into()),
        shortcut: None,
        disabled,
        action: Box::new(action),
    }
}

fn is_current(target: &BranchMenuTarget, current: Option<&str>) -> bool {
    matches!(target, BranchMenuTarget::Local { name } if Some(name.as_str()) == current)
}

/// Builds the entries and the tooltips of the greyed-out ones, keyed by
/// item index.
pub(super) fn drop_items(
    repo_id: RepoId,
    source: &BranchDropEnd<'_>,
    target: &BranchDropEnd<'_>,
    facts: &BranchDropFacts<'_>,
    language: Language,
) -> (Vec<ContextMenuItem>, FxHashMap<usize, SharedString>) {
    let source_name = source.target.display_name();
    let target_name = target.target.display_name();
    let mut items = vec![
        ContextMenuItem::Header(language.text("Drop branch", "拖放分支").into()),
        ContextMenuItem::Label(format!("{source_name} → {target_name}").into()),
        ContextMenuItem::Separator,
    ];
    let mut tooltips: FxHashMap<usize, SharedString> = FxHashMap::default();
    let source_is_current = is_current(source.target, facts.current);
    let target_is_current = is_current(target.target, facts.current);
    let merge_label = || match language {
        Language::English => format!("Merge {source_name} into {target_name}"),
        Language::TraditionalChinese => format!("將 {source_name} 合併至 {target_name}"),
    };
    let rebase_label = || match language {
        Language::English => format!("Rebase {source_name} onto {target_name}"),
        Language::TraditionalChinese => format!("將 {source_name} 變基至 {target_name}"),
    };

    if target_is_current {
        items.push(entry(
            merge_label(),
            "icons/swap.svg",
            !facts.merge_allowed,
            ContextMenuAction::MergeRef {
                repo_id,
                reference: source.reference.clone(),
            },
        ));
        items.push(entry(
            match language {
                Language::English => format!("Squash {source_name} into {target_name}"),
                Language::TraditionalChinese => {
                    format!("將 {source_name} 壓縮合併至 {target_name}")
                }
            },
            "icons/arrow_right.svg",
            !facts.merge_allowed,
            ContextMenuAction::SquashRef {
                repo_id,
                reference: source.reference.clone(),
            },
        ));
    } else if source_is_current {
        items.push(entry(
            rebase_label(),
            "icons/arrow_up.svg",
            facts.history_rewrite_busy,
            ContextMenuAction::OpenPopover {
                kind: PopoverKind::RebaseOntoConfirm {
                    repo_id,
                    onto: target.reference.clone(),
                },
            },
        ));
        items.push(entry(
            match language {
                Language::English => format!("Interactive rebase {source_name} onto {target_name}"),
                Language::TraditionalChinese => {
                    format!("將 {source_name} 互動式變基至 {target_name}")
                }
            },
            "icons/refresh.svg",
            facts.history_rewrite_busy,
            ContextMenuAction::LoadInteractiveRebaseSetup {
                repo_id,
                base: target.reference.clone(),
            },
        ));
        if let BranchMenuTarget::Remote { remote, branch } = target.target {
            let push_label = match language {
                Language::English => format!("Push {source_name} to {target_name}"),
                Language::TraditionalChinese => format!("將 {source_name} 推送至 {target_name}"),
            };
            let is_upstream = facts.current_upstream == Some((remote.as_str(), branch.as_str()));
            let first_push_to_namesake =
                facts.current_upstream.is_none() && Some(branch.as_str()) == facts.current;
            // git-annex runs Push itself, without the upstream prompt.
            if is_upstream || (first_push_to_namesake && facts.annex_pushes) {
                items.push(entry(
                    push_label,
                    "icons/arrow_up.svg",
                    false,
                    ContextMenuAction::Push { repo_id },
                ));
            } else if first_push_to_namesake {
                // The first push of a branch to its namesake also sets it as
                // the upstream, through the usual first-push prompt.
                items.push(entry(
                    push_label,
                    "icons/arrow_up.svg",
                    false,
                    ContextMenuAction::OpenPopover {
                        kind: PopoverKind::PushSetUpstreamPrompt {
                            repo_id,
                            remote: remote.clone(),
                            configure_only_for: None,
                        },
                    },
                ));
            } else {
                tooltips.insert(
                    items.len(),
                    language.text(
                        "Push goes to the branch's upstream; a branch without one can push to the remote branch of its own name",
                        "推送目的地是上游分支。尚未設定上游的分支可推送至同名的遠端分支。",
                    )
                    .into(),
                );
                items.push(entry(
                    push_label,
                    "icons/arrow_up.svg",
                    true,
                    ContextMenuAction::Push { repo_id },
                ));
            }
        }
    } else {
        // Neither end is checked out: say which one needs to be.
        let merge_reason = match target.target {
            BranchMenuTarget::Local { .. } => match language {
                Language::English => format!("Check out {target_name} first: merges go into the checked-out branch"),
                Language::TraditionalChinese => format!("請先切換至 {target_name}。合併會套用至目前的分支。"),
            },
            BranchMenuTarget::Remote { .. } => language
                .text(
                    "Merges go into the checked-out branch, and a remote branch cannot be checked out",
                    "合併會套用至目前的分支。遠端分支無法直接切換。",
                )
                .to_string(),
        };
        tooltips.insert(items.len(), merge_reason.into());
        items.push(entry(
            merge_label(),
            "icons/swap.svg",
            true,
            ContextMenuAction::MergeRef {
                repo_id,
                reference: source.reference.clone(),
            },
        ));
        let rebase_reason = match source.target {
            BranchMenuTarget::Local { .. } => match language {
                Language::English => format!("Check out {source_name} first: a rebase moves the checked-out branch"),
                Language::TraditionalChinese => format!("請先切換至 {source_name}。變基會移動目前的分支。"),
            },
            BranchMenuTarget::Remote { .. } => language
                .text(
                    "A rebase moves the checked-out branch, and a remote branch cannot be checked out",
                    "變基會移動目前的分支。遠端分支無法直接切換。",
                )
                .to_string(),
        };
        tooltips.insert(items.len(), rebase_reason.into());
        items.push(entry(
            rebase_label(),
            "icons/arrow_up.svg",
            true,
            ContextMenuAction::OpenPopover {
                kind: PopoverKind::RebaseOntoConfirm {
                    repo_id,
                    onto: target.reference.clone(),
                },
            },
        ));
    }
    (items, tooltips)
}

/// The exact reference Git actions use for `target`, as the branch menu
/// resolves it: the name for a local branch, the loaded tip for a remote one.
fn action_reference(repo: Option<&RepoState>, target: &BranchMenuTarget) -> String {
    match target {
        BranchMenuTarget::Local { name } => name.clone(),
        BranchMenuTarget::Remote { remote, branch } => repo
            .and_then(|repo| repo.remote_branches.ready())
            .and_then(|branches| {
                branches
                    .iter()
                    .find(|candidate| candidate.remote == *remote && candidate.name == *branch)
                    .map(|candidate| candidate.target.as_ref().to_string())
            })
            .unwrap_or_else(|| target.display_name()),
    }
}

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    source: &BranchMenuTarget,
    target: &BranchMenuTarget,
    language: Language,
) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|r| r.id == repo_id);
    let current = repo
        .and_then(|repo| repo.head_branch.ready())
        .map(String::as_str)
        .filter(|name| !name.is_empty() && *name != "HEAD");
    let current_upstream = repo.and_then(|repo| {
        let branches = repo.branches.ready()?;
        let upstream = branches
            .iter()
            .find(|branch| Some(branch.name.as_str()) == current)?
            .upstream
            .as_ref()?;
        Some((upstream.remote.as_str(), upstream.branch.as_str()))
    });
    let facts = BranchDropFacts {
        current,
        current_upstream,
        merge_allowed: super::super::merge_commit_confirm::merge_commit_is_allowed(
            repo,
            &this.state.large_file_settings,
        ),
        history_rewrite_busy: repo.is_some_and(|repo| repo.history_rewrite_busy()),
        annex_pushes: repo
            .is_some_and(|repo| repo.annex_takes_over_pull_push(&this.state.large_file_settings)),
    };
    let (items, tooltips) = drop_items(
        repo_id,
        &BranchDropEnd {
            target: source,
            reference: action_reference(repo, source),
        },
        &BranchDropEnd {
            target,
            reference: action_reference(repo, target),
        },
        &facts,
        language,
    );
    ContextMenuModel::new(items).with_entry_tooltips(tooltips)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(target: &BranchMenuTarget) -> BranchDropEnd<'_> {
        BranchDropEnd {
            target,
            reference: target.display_name(),
        }
    }

    fn facts<'a>(
        current: Option<&'a str>,
        upstream: Option<(&'a str, &'a str)>,
    ) -> BranchDropFacts<'a> {
        BranchDropFacts {
            current,
            current_upstream: upstream,
            merge_allowed: true,
            history_rewrite_busy: false,
            annex_pushes: false,
        }
    }

    /// `(label, disabled)` of each entry.
    fn entries(items: &[ContextMenuItem]) -> Vec<(String, bool)> {
        items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry {
                    label, disabled, ..
                } => Some((label.to_string(), *disabled)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn dropping_onto_the_current_branch_merges_into_it() {
        let feature = BranchMenuTarget::local("feature");
        let main = BranchMenuTarget::local("main");
        let (items, tooltips) = drop_items(
            RepoId(1),
            &end(&feature),
            &end(&main),
            &facts(Some("main"), None),
            Language::English,
        );
        assert_eq!(
            entries(&items),
            vec![
                ("Merge feature into main".to_string(), false),
                ("Squash feature into main".to_string(), false),
            ]
        );
        assert!(tooltips.is_empty());
        assert!(items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { action, .. }
                if matches!(action.as_ref(), ContextMenuAction::MergeRef { reference, .. } if reference == "feature")
        )));
    }

    #[test]
    fn traditional_chinese_branch_drop_preserves_direction_and_git_reference() {
        let source = BranchMenuTarget::remote("origin", "feature");
        let target = BranchMenuTarget::local("main");
        let source = BranchDropEnd {
            target: &source,
            reference: "deadbeef".to_owned(),
        };
        let (items, tooltips) = drop_items(
            RepoId(1),
            &source,
            &end(&target),
            &facts(Some("main"), None),
            Language::TraditionalChinese,
        );
        assert!(tooltips.is_empty());
        assert!(items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { label, action, disabled: false, .. }
                if label.as_ref() == "將 origin/feature 合併至 main"
                    && matches!(action.as_ref(), ContextMenuAction::MergeRef { reference, .. } if reference == "deadbeef")
        )));
    }

    #[test]
    fn dropping_the_current_branch_rebases_it_and_pushes_to_its_upstream() {
        let main = BranchMenuTarget::local("main");
        let origin_main = BranchMenuTarget::remote("origin", "main");
        let (items, _) = drop_items(
            RepoId(1),
            &end(&main),
            &end(&origin_main),
            &facts(Some("main"), Some(("origin", "main"))),
            Language::English,
        );
        assert_eq!(
            entries(&items),
            vec![
                ("Rebase main onto origin/main".to_string(), false),
                (
                    "Interactive rebase main onto origin/main".to_string(),
                    false
                ),
                ("Push main to origin/main".to_string(), false),
            ]
        );
        assert!(items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { action, .. }
                if matches!(action.as_ref(), ContextMenuAction::Push { .. })
        )));
    }

    #[test]
    fn first_push_to_a_namesake_opens_the_upstream_prompt() {
        let feature = BranchMenuTarget::local("feature");
        let origin_feature = BranchMenuTarget::remote("origin", "feature");
        let (items, _) = drop_items(
            RepoId(1),
            &end(&feature),
            &end(&origin_feature),
            &facts(Some("feature"), None),
            Language::English,
        );
        assert!(items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { action, disabled: false, .. }
                if matches!(
                    action.as_ref(),
                    ContextMenuAction::OpenPopover {
                        kind: PopoverKind::PushSetUpstreamPrompt { remote, configure_only_for: None, .. }
                    } if remote == "origin"
                )
        )));
    }

    #[test]
    fn push_to_another_remote_branch_is_offered_disabled_with_a_reason() {
        let feature = BranchMenuTarget::local("feature");
        let origin_main = BranchMenuTarget::remote("origin", "main");
        let (items, tooltips) = drop_items(
            RepoId(1),
            &end(&feature),
            &end(&origin_main),
            &facts(Some("feature"), Some(("origin", "feature"))),
            Language::English,
        );
        let push_ix = items
            .iter()
            .position(|item| matches!(item, ContextMenuItem::Entry { label, .. } if label.starts_with("Push")))
            .expect("a push entry");
        assert!(matches!(
            &items[push_ix],
            ContextMenuItem::Entry { disabled: true, .. }
        ));
        assert!(tooltips.contains_key(&push_ix));
    }

    #[test]
    fn branches_that_are_not_checked_out_are_explained_not_switched() {
        let a = BranchMenuTarget::local("a");
        let b = BranchMenuTarget::local("b");
        let (items, tooltips) = drop_items(
            RepoId(1),
            &end(&a),
            &end(&b),
            &facts(Some("main"), None),
            Language::English,
        );
        assert_eq!(
            entries(&items),
            vec![
                ("Merge a into b".to_string(), true),
                ("Rebase a onto b".to_string(), true),
            ]
        );
        assert_eq!(tooltips.len(), 2);
    }

    #[test]
    fn busy_repositories_grey_out_merges_and_rebases() {
        let feature = BranchMenuTarget::local("feature");
        let main = BranchMenuTarget::local("main");
        let busy = BranchDropFacts {
            current: Some("main"),
            current_upstream: None,
            merge_allowed: false,
            history_rewrite_busy: true,
            annex_pushes: false,
        };
        let (items, _) = drop_items(
            RepoId(1),
            &end(&feature),
            &end(&main),
            &busy,
            Language::English,
        );
        assert!(entries(&items).iter().all(|(_, disabled)| *disabled));
        let (items, _) = drop_items(
            RepoId(1),
            &end(&main),
            &end(&feature),
            &busy,
            Language::English,
        );
        assert!(entries(&items).iter().all(|(_, disabled)| *disabled));
    }

    #[test]
    fn first_push_under_git_annex_pushes_without_the_upstream_prompt() {
        let feature = BranchMenuTarget::local("feature");
        let origin_feature = BranchMenuTarget::remote("origin", "feature");
        let annex = BranchDropFacts {
            annex_pushes: true,
            ..facts(Some("feature"), None)
        };
        let (items, _) = drop_items(
            RepoId(1),
            &end(&feature),
            &end(&origin_feature),
            &annex,
            Language::English,
        );
        assert!(items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { action, disabled: false, .. }
                if matches!(action.as_ref(), ContextMenuAction::Push { .. })
        )));
    }

    #[test]
    fn a_remote_target_explains_that_it_cannot_be_checked_out() {
        let a = BranchMenuTarget::local("a");
        let origin_b = BranchMenuTarget::remote("origin", "b");
        let (_, tooltips) = drop_items(
            RepoId(1),
            &end(&a),
            &end(&origin_b),
            &facts(Some("main"), None),
            Language::English,
        );
        assert!(
            tooltips
                .values()
                .any(|text| text.contains("a remote branch cannot be checked out"))
        );
    }
}
