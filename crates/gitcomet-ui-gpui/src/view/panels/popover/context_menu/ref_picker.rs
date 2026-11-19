//! The palette's pick-a-ref menus for deletions. Each row opens the
//! deletion's confirmation; nothing is deleted from the menu itself.
use super::*;

pub(super) fn model(
    this: &PopoverHost,
    repo_id: RepoId,
    purpose: RefPickerPurpose,
) -> ContextMenuModel {
    let repo = this.state.repos.iter().find(|repo| repo.id == repo_id);
    let (title, empty) = match purpose {
        RefPickerPurpose::DeleteTag => ("Delete Tag", "No tags"),
        RefPickerPurpose::DeleteRemoteBranch => ("Delete Remote Branch", "No remote branches"),
    };
    let mut items = vec![
        ContextMenuItem::Header(title.into()),
        ContextMenuItem::Separator,
    ];
    let rows: Vec<(String, PopoverKind)> = match purpose {
        RefPickerPurpose::DeleteTag => {
            let mut names: Vec<String> = repo
                .and_then(|repo| repo.tags.ready())
                .map(|tags| tags.iter().map(|tag| tag.name.clone()).collect())
                .unwrap_or_default();
            names.sort_by_key(|name| name.to_lowercase());
            names
                .into_iter()
                .map(|name| {
                    (
                        name.clone(),
                        PopoverKind::DeleteTagConfirm {
                            repo_id,
                            name,
                            remote: None,
                        },
                    )
                })
                .collect()
        }
        RefPickerPurpose::DeleteRemoteBranch => {
            let mut branches: Vec<(String, String)> = repo
                .and_then(|repo| repo.remote_branches.ready())
                .map(|branches| {
                    branches
                        .iter()
                        .filter(|branch| {
                            branch.name != "HEAD"
                                && !repo.is_some_and(|repo| repo.hides_annex_ref(&branch.name))
                        })
                        .map(|branch| (branch.remote.clone(), branch.name.clone()))
                        .collect()
                })
                .unwrap_or_default();
            branches.sort();
            branches
                .into_iter()
                .map(|(remote, branch)| {
                    (
                        format!("{remote}/{branch}"),
                        PopoverKind::remote(
                            repo_id,
                            RemotePopoverKind::DeleteBranchConfirm { remote, branch },
                        ),
                    )
                })
                .collect()
        }
    };
    if rows.is_empty() {
        items.push(ContextMenuItem::Label(empty.into()));
        return ContextMenuModel::new(items);
    }
    for (label, kind) in rows {
        items.push(ContextMenuItem::Entry {
            label: label.into(),
            icon: Some("icons/trash.svg".into()),
            shortcut: None,
            disabled: false,
            action: Box::new(ContextMenuAction::OpenPopover { kind }),
        });
    }
    ContextMenuModel::new(items)
}
