//! Dragging a branch onto another, as in GitKraken: the drop opens
//! a menu of merges, rebases and pushes between the two branches.

use super::*;

/// The branch being dragged, with the repository it belongs to so a drop
/// in another repository is refused.
#[derive(Clone)]
pub(in crate::view) struct BranchDrag {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) lifetime: u64,
    pub(in crate::view) target: BranchMenuTarget,
}

impl BranchDrag {
    /// Both surfaces validate against the live store, not their last frame.
    pub(in crate::view) fn live_refs(
        state: &AppState,
        repo_id: RepoId,
        lifetime: u64,
        targets: &[&BranchMenuTarget],
    ) -> bool {
        state.active_repo == Some(repo_id)
            && state
                .repos
                .iter()
                .find(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
                .is_some_and(|repo| {
                    targets.iter().all(|target| match target {
                        BranchMenuTarget::Local { name } => {
                            repo.branches.ready().is_some_and(|branches| {
                                branches.iter().any(|branch| branch.name == *name)
                            })
                        }
                        BranchMenuTarget::Remote { remote, branch } => {
                            repo.remote_branches.ready().is_some_and(|branches| {
                                branches.iter().any(|candidate| {
                                    candidate.remote == *remote && candidate.name == *branch
                                })
                            })
                        }
                    })
                })
    }

    /// Whether dropping on `target` in `repo_id` makes sense: another branch
    /// of the same repository.
    pub(in crate::view) fn can_drop_on(&self, repo_id: RepoId, target: &BranchMenuTarget) -> bool {
        self.repo_id == repo_id && self.target != *target
    }

    pub(in crate::view) fn can_drop_live(
        &self,
        state: &AppState,
        repo_id: RepoId,
        lifetime: u64,
        target: &BranchMenuTarget,
    ) -> bool {
        self.lifetime == lifetime
            && self.can_drop_on(repo_id, target)
            && Self::live_refs(state, repo_id, lifetime, &[&self.target, target])
    }
}

/// The bubble that follows the cursor while a branch is dragged.
pub(in crate::view) struct BranchDragPreview {
    label: SharedString,
    theme: AppTheme,
    grab_offset: Point<Pixels>,
}

impl BranchDragPreview {
    pub(in crate::view) fn new(
        drag: &BranchDrag,
        theme: AppTheme,
        grab_offset: Point<Pixels>,
    ) -> Self {
        Self {
            label: drag.target.display_name().into(),
            theme,
            grab_offset,
        }
    }
}

impl Render for BranchDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let scale = ui_scale::current(cx).percent;
        let scaled = |value| ui_scale::design_px_from_percent(value, scale);
        // GPUI subtracts the row's grab offset; cancel it so the bubble keeps
        // a fixed distance from the cursor wherever the row was grabbed.
        div()
            .pl(self.grab_offset.x + scaled(12.0))
            .pt(self.grab_offset.y + scaled(16.0))
            .child(
                div()
                    .debug_selector(|| "sidebar_branch_drag_preview".into())
                    .flex()
                    .items_center()
                    .gap(scaled(6.0))
                    .max_w(scaled(320.0))
                    .px(scaled(8.0))
                    .py(scaled(4.0))
                    .rounded(scaled(theme.radii.row.max(6.0)))
                    .border_1()
                    .border_color(theme.colors.stroke.default)
                    .bg(theme.colors.surface.raised)
                    .text_color(theme.colors.foreground.primary)
                    .text_size(scaled(12.0))
                    .child(crate::view::icons::svg_icon(
                        "icons/git_branch.svg",
                        theme.colors.foreground.secondary,
                        scaled(14.0),
                    ))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(self.label.clone()),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> RepoState {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            gitcomet_core::domain::RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/branch-drag-validation"),
            },
        );
        repo.branches = Loadable::Ready(Arc::new(
            ["a", "b"]
                .into_iter()
                .map(|name| gitcomet_core::domain::Branch {
                    name: name.into(),
                    target: gitcomet_core::domain::CommitId("same-tip".into()),
                    upstream: None,
                    divergence: None,
                })
                .collect(),
        ));
        repo.remote_branches =
            Loadable::Ready(Arc::new(vec![gitcomet_core::domain::RemoteBranch {
                remote: "origin".into(),
                name: "b".into(),
                target: gitcomet_core::domain::CommitId("same-tip".into()),
            }]));
        repo
    }

    #[test]
    fn live_drop_gate_rejects_stale_typed_refs_active_repo_and_source_lifetime() {
        let repo = repository();
        let lifetime = repo.lifetime();
        let drag = BranchDrag {
            repo_id: repo.id,
            lifetime,
            target: BranchMenuTarget::local("a"),
        };
        let local_target = BranchMenuTarget::local("b");
        let remote_target = BranchMenuTarget::remote("origin", "b");
        let mut state = AppState {
            repos: vec![repo],
            active_repo: Some(RepoId(1)),
            ..AppState::test_default()
        };
        assert!(drag.can_drop_live(&state, RepoId(1), lifetime, &local_target));
        assert!(drag.can_drop_live(&state, RepoId(1), lifetime, &remote_target));
        state.active_repo = None;
        assert!(!drag.can_drop_live(&state, RepoId(1), lifetime, &local_target));
        state.active_repo = Some(RepoId(1));
        let valid_branches = state.repos[0].branches.clone();
        state.repos[0].branches = Loadable::Ready(Arc::new(Vec::new()));
        assert!(
            !drag.can_drop_live(&state, RepoId(1), lifetime, &remote_target),
            "missing source"
        );
        state.repos[0].branches = valid_branches;
        state.repos[0].remote_branches = Loadable::Ready(Arc::new(Vec::new()));
        assert!(
            !drag.can_drop_live(&state, RepoId(1), lifetime, &remote_target),
            "missing typed target"
        );
        assert!(drag.can_drop_live(&state, RepoId(1), lifetime, &local_target));

        // Mode transitions can replace a lifetime without changing repo ID,
        // names or tips. A newly painted destination must reject an old drag.
        let replacement = repository();
        let replacement_lifetime = replacement.lifetime();
        assert_ne!(lifetime, replacement_lifetime);
        state.repos[0] = replacement;
        assert!(BranchDrag::live_refs(
            &state,
            RepoId(1),
            replacement_lifetime,
            &[&drag.target, &local_target]
        ));
        assert!(!drag.can_drop_live(&state, RepoId(1), replacement_lifetime, &local_target));
    }

    #[test]
    fn a_branch_drops_only_on_other_branches_of_its_repository() {
        let drag = BranchDrag {
            repo_id: RepoId(1),
            lifetime: 1,
            target: BranchMenuTarget::local("feature"),
        };
        assert!(drag.can_drop_on(RepoId(1), &BranchMenuTarget::local("main")));
        assert!(drag.can_drop_on(RepoId(1), &BranchMenuTarget::remote("origin", "feature")));
        assert!(!drag.can_drop_on(RepoId(1), &BranchMenuTarget::local("feature")));
        assert!(!drag.can_drop_on(RepoId(2), &BranchMenuTarget::local("main")));
    }
}
