//! Dragging a sidebar branch onto another, as in GitKraken: the drop opens
//! a menu of merges, rebases and pushes between the two branches.

use super::*;

/// The branch being dragged, with the repository it belongs to so a drop
/// in another repository's sidebar is refused.
#[derive(Clone)]
pub(in crate::view) struct SidebarBranchDrag {
    pub(in crate::view) repo_id: RepoId,
    pub(in crate::view) target: BranchMenuTarget,
}

impl SidebarBranchDrag {
    /// Whether dropping on `target` in `repo_id` makes sense: another branch
    /// of the same repository.
    pub(in crate::view) fn can_drop_on(&self, repo_id: RepoId, target: &BranchMenuTarget) -> bool {
        self.repo_id == repo_id && self.target != *target
    }
}

/// The bubble that follows the cursor while a branch is dragged.
pub(super) struct SidebarBranchDragPreview {
    label: SharedString,
    theme: AppTheme,
    grab_offset: gpui::Point<Pixels>,
}

impl SidebarBranchDragPreview {
    pub(super) fn new(
        drag: &SidebarBranchDrag,
        theme: AppTheme,
        grab_offset: gpui::Point<Pixels>,
    ) -> Self {
        Self {
            label: drag.target.display_name().into(),
            theme,
            grab_offset,
        }
    }
}

impl Render for SidebarBranchDragPreview {
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

    #[test]
    fn a_branch_drops_only_on_other_branches_of_its_repository() {
        let drag = SidebarBranchDrag {
            repo_id: RepoId(1),
            target: BranchMenuTarget::local("feature"),
        };
        assert!(drag.can_drop_on(RepoId(1), &BranchMenuTarget::local("main")));
        assert!(drag.can_drop_on(RepoId(1), &BranchMenuTarget::remote("origin", "feature")));
        assert!(!drag.can_drop_on(RepoId(1), &BranchMenuTarget::local("feature")));
        assert!(!drag.can_drop_on(RepoId(2), &BranchMenuTarget::local("main")));
    }
}
