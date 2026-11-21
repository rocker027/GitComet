use super::status_nav::status_navigation_context_for_repo;
use super::*;
use crate::view::panes::diff::helpers::state::{
    NextFileAdvance, OpenFileChange, StageFileTransfer,
};
use gitcomet_state::model::LoadKind;

/// What a dispatch did to the open file, carried to the deferred arming.
struct ArmedChange {
    repo_id: RepoId,
    path: std::path::PathBuf,
    area: DiffArea,
    change: OpenFileChange,
    /// `ops_rev` before the action began.
    ops_rev: u64,
}

impl DiffRenderer {
    /// The explicit file button follows that file, independent of the
    /// next-file preference used while reviewing individual changes.
    pub(super) fn stage_shown_file_preserving_diff(
        &mut self,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(repo) = self.active_repo() else {
            return false;
        };
        if self.hosted_decor.is_some()
            || self.shows_linked_worktree()
            || self.is_inline_submodule_diff_active()
            || repo.local_actions_in_flight > 0
            || self.stage_file_transfer.is_some()
        {
            return false;
        }
        let diff = self.bound_diff_state(repo);
        if diff.content_preview {
            return false;
        }
        let Some(DiffTarget::WorkingTree { path, area, .. }) = diff.diff_target.as_ref() else {
            return false;
        };
        let transfer = StageFileTransfer {
            repo_id: repo.id,
            repo_lifetime: repo.lifetime(),
            path: path.clone(),
            from: *area,
            ops_rev: repo.ops_rev,
            canvas_requests: repo.navigation.canvas_requests,
            opened: false,
            scroll_offsets: [&self.diff_scroll, &self.diff_split_right_scroll]
                .map(|scroll| scroll.0.borrow().base_handle.offset()),
        };
        // Preserve the existing batch operation when the button represents
        // several selected paths.
        if self
            .status_selection_for_shortcut(transfer.repo_id, transfer.from, &transfer.path, cx)
            .is_some()
        {
            return self.toggle_stage_shown_file(window, cx);
        }
        let consumes_selection = self.status_single_selection_for_shortcut(
            transfer.repo_id,
            transfer.from,
            &transfer.path,
            cx,
        );
        if self.confirm_stage_conflict_markers(
            transfer.repo_id,
            transfer.from,
            vec![transfer.path.clone()],
            consumes_selection,
            window,
            cx,
        ) {
            return true;
        }
        if consumes_selection {
            self.clear_status_selection_for_shortcut(transfer.repo_id, cx);
        }
        self.begin_stage_file_transfer(transfer);
        true
    }

    /// Row actions have already resolved selection and confirmed conflicts.
    /// Follow only the exact open working-tree diff, without querying the
    /// details pane again while its row callback is running.
    pub(in crate::view) fn stage_open_file_from_status(
        &mut self,
        repo_id: RepoId,
        path: &std::path::Path,
        area: DiffArea,
    ) -> bool {
        let Some(repo) = self.active_repo().filter(|repo| repo.id == repo_id) else {
            return false;
        };
        if self.hosted_decor.is_some()
            || self.shows_linked_worktree()
            || self.is_inline_submodule_diff_active()
            || repo.local_actions_in_flight > 0
            || self.stage_file_transfer.is_some()
        {
            return false;
        }
        let diff = self.bound_diff_state(repo);
        if diff.content_preview
            || !matches!(diff.diff_target.as_ref(), Some(DiffTarget::WorkingTree { path: shown, area: shown_area, .. })
                if shown == path && *shown_area == area)
        {
            return false;
        }
        let transfer = StageFileTransfer {
            repo_id,
            repo_lifetime: repo.lifetime(),
            path: path.to_path_buf(),
            from: area,
            ops_rev: repo.ops_rev,
            canvas_requests: repo.navigation.canvas_requests,
            opened: false,
            scroll_offsets: [&self.diff_scroll, &self.diff_split_right_scroll]
                .map(|scroll| scroll.0.borrow().base_handle.offset()),
        };
        self.begin_stage_file_transfer(transfer);
        true
    }

    fn begin_stage_file_transfer(&mut self, transfer: StageFileTransfer) {
        let msg = match transfer.from {
            DiffArea::Unstaged => Msg::StagePath {
                repo_id: transfer.repo_id,
                path: transfer.path.clone(),
            },
            DiffArea::Staged => Msg::UnstagePath {
                repo_id: transfer.repo_id,
                path: transfer.path.clone(),
            },
        };
        self.next_file_advance = None;
        self.stage_file_transfer = Some(transfer);
        self.store.dispatch(msg);
    }

    pub(in crate::view) fn resolve_stage_file_transfer(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(mut transfer) = self.stage_file_transfer.take() else {
            return;
        };
        let Some(repo) = self.active_repo().filter(|repo| {
            repo.id == transfer.repo_id && repo.lifetime() == transfer.repo_lifetime
        }) else {
            return;
        };
        let to = match transfer.from {
            DiffArea::Unstaged => DiffArea::Staged,
            DiffArea::Staged => DiffArea::Unstaged,
        };
        let diff = self.bound_diff_state(repo);
        let showing_source = diff.diff_target.as_ref().is_some_and(|target| {
            matches!(target, DiffTarget::WorkingTree { path, area, .. }
                if path == &transfer.path && *area == transfer.from)
        });
        let showing_destination = diff.diff_target.as_ref().is_some_and(|target| {
            matches!(target, DiffTarget::WorkingTree { path, area, .. }
                if path == &transfer.path && *area == to)
        });
        // Our deferred SelectDiff is one canvas request too. Until it is
        // applied, old source snapshots still belong to the original request.
        let expected_canvas_requests = if transfer.opened && showing_destination {
            transfer.canvas_requests.wrapping_add(1)
        } else {
            transfer.canvas_requests
        };
        if repo.navigation.canvas_requests != expected_canvas_requests {
            return;
        }
        if transfer.opened {
            if !showing_destination {
                if showing_source || diff.diff_target.is_none() {
                    self.stage_file_transfer = Some(transfer);
                }
                return;
            }
            if matches!(diff.diff, Loadable::Loading | Loadable::NotLoaded) {
                self.stage_file_transfer = Some(transfer);
                return;
            }
            for (scroll, offset) in [&self.diff_scroll, &self.diff_split_right_scroll]
                .into_iter()
                .zip(transfer.scroll_offsets)
            {
                scroll.0.borrow().base_handle.set_offset(offset);
            }
            self.diff_autoscroll_pending = false;
            // The root or details pane may still be updating this renderer.
            // Defer both projection lookup and reveal until their borrows end.
            self.commands.defer(
                crate::view::panes::diff::services::DiffCommand::RevealStatus {
                    section: match to {
                        DiffArea::Staged => StatusSection::Staged,
                        DiffArea::Unstaged => StatusSection::CombinedUnstaged,
                    },
                    path: transfer.path,
                    after_transfer: Some((
                        transfer.repo_id,
                        transfer.repo_lifetime,
                        expected_canvas_requests,
                    )),
                },
                cx,
            );
            return;
        }
        // A new file selection wins. `None` is expected when the reducer
        // retires the source lane after the whole-file operation succeeds.
        if diff.diff_target.is_some() && !showing_source {
            return;
        }
        let status_loading = repo.loads_in_flight.is_in_flight(LoadKind::WorktreeStatus)
            || repo.loads_in_flight.is_in_flight(LoadKind::StagedStatus);
        if repo.ops_rev == transfer.ops_rev || repo.local_actions_in_flight > 0 || status_loading {
            self.stage_file_transfer = Some(transfer);
            return;
        }
        if repo.feedback.last_error.is_some()
            || repo.status_entry_for_path(to, &transfer.path).is_none()
        {
            return;
        }
        let repo_id = transfer.repo_id;
        let repo_lifetime = transfer.repo_lifetime;
        let path = transfer.path.clone();
        let canvas_requests = transfer.canvas_requests;
        transfer.opened = true;
        self.stage_file_transfer = Some(transfer);
        let pane = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = pane.update(cx, |pane, _| {
                let Some(current) = pane.store.snapshot() else {
                    pane.stage_file_transfer = None;
                    return;
                };
                let Some(repo) = current.repos.iter().find(|repo| {
                    repo.id == repo_id
                        && repo.lifetime() == repo_lifetime
                        && current.active_repo == Some(repo_id)
                }) else {
                    pane.stage_file_transfer = None;
                    return;
                };
                if repo.navigation.canvas_requests != canvas_requests
                    || repo.diff_state.diff_target.as_ref().is_some_and(|target| {
                        !matches!(target, DiffTarget::WorkingTree { path: shown, area, .. }
                            if shown == &path && *area != to)
                    })
                    || !pane
                        .stage_file_transfer
                        .as_ref()
                        .is_some_and(|pending| pending.opened)
                {
                    pane.stage_file_transfer = None;
                    return;
                }
                pane.store.dispatch(Msg::SelectDiff {
                    repo_id,
                    target: DiffTarget::working_tree(path, to),
                });
            });
        });
    }

    pub(in crate::view) fn set_open_next_file_after_last_change(&mut self, enabled: bool) {
        self.open_next_file_after_last_change = enabled;
        if !enabled {
            self.next_file_advance = None;
        }
    }

    /// Sends a Stage, Unstage or Discard, and arms the next-file advance when
    /// it changes the open working-tree file.
    pub(in crate::view) fn dispatch_open_file_change(
        &mut self,
        msg: Msg,
        cx: &mut gpui::Context<Self>,
    ) {
        let armed = self.arm_for(&msg);
        self.store.dispatch(msg);
        let Some(armed) = armed else {
            return;
        };
        // Deferred: the list order is read from the root view, which may be
        // updating us right now (keys pressed outside the diff).
        let pane = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = pane.update(cx, |pane, cx| pane.arm_next_file_advance(armed, cx));
        });
    }

    /// What `msg` does to the open working-tree file, or `None` when it
    /// leaves that file alone or the advance is off.
    fn arm_for(&self, msg: &Msg) -> Option<ArmedChange> {
        if !self.open_next_file_after_last_change || self.hosted_decor.is_some() {
            return None;
        }
        let repo = self.active_repo()?;
        let Some(DiffTarget::WorkingTree {
            path: open, area, ..
        }) = self.bound_diff_state(repo).diff_target.as_ref()
        else {
            return None;
        };
        let change = match msg {
            Msg::StageHunk { repo_id, .. }
            | Msg::UnstageHunk { repo_id, .. }
            | Msg::ApplyWorktreePatch { repo_id, .. }
            | Msg::ApplyPreparedDiffPatch { repo_id, .. } => {
                (*repo_id == repo.id).then_some(OpenFileChange::Part)
            }
            Msg::DiscardWorktreeChangesPath { repo_id, path } => {
                (*repo_id == repo.id && path == open).then_some(OpenFileChange::WholeFile)
            }
            Msg::DiscardWorktreeChangesPaths { repo_id, paths } => {
                (*repo_id == repo.id && paths.contains(open)).then_some(OpenFileChange::WholeFile)
            }
            _ => None,
        }?;
        Some(ArmedChange {
            repo_id: repo.id,
            path: open.clone(),
            area: *area,
            change,
            ops_rev: repo.ops_rev,
        })
    }

    fn arm_next_file_advance(&mut self, armed: ArmedChange, cx: &mut gpui::Context<Self>) {
        self.next_file_advance = None;
        let view = self.active_change_tracking_view(cx);
        let order = self.active_status_section_order(armed.repo_id, view, cx);
        let Some(repo) = self.active_repo().filter(|repo| repo.id == armed.repo_id) else {
            return;
        };
        let target = DiffTarget::working_tree(armed.path.clone(), armed.area);
        let Some((section, next)) =
            status_navigation_context_for_repo(repo, &target, view, order.as_deref()).and_then(
                |navigation| Some((navigation.section(), navigation.next_or_prev_path()?)),
            )
        else {
            return;
        };
        self.next_file_advance = Some(NextFileAdvance {
            repo_id: armed.repo_id,
            path: armed.path,
            area: armed.area,
            change: armed.change,
            next,
            section,
            ops_rev: armed.ops_rev,
        });
    }

    /// Runs on every new pane input. Once the armed action finished and the
    /// status after it loaded, decides once: opens the neighbour if the file
    /// has left its list. Showing another file disarms it.
    pub(in crate::view) fn resolve_next_file_advance(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(advance) = self.next_file_advance.take() else {
            return;
        };
        let Some(repo) = self.active_repo().filter(|repo| repo.id == advance.repo_id) else {
            return;
        };
        let still_shown = match self.bound_diff_state(repo).diff_target.as_ref() {
            Some(DiffTarget::WorkingTree { path, area, .. }) => {
                *path == advance.path && *area == advance.area
            }
            Some(_) => false,
            None => advance.change == OpenFileChange::WholeFile,
        };
        if !still_shown {
            return;
        }
        let status_loading = repo.loads_in_flight.is_in_flight(LoadKind::WorktreeStatus)
            || repo.loads_in_flight.is_in_flight(LoadKind::StagedStatus);
        if repo.ops_rev == advance.ops_rev || repo.local_actions_in_flight > 0 || status_loading {
            self.next_file_advance = Some(advance);
            return;
        }
        if repo
            .status_entry_for_path(advance.area, &advance.path)
            .is_some()
        {
            return;
        }
        let Some(next) = repo.status_entry_for_path(advance.area, &advance.next) else {
            return;
        };
        let is_conflicted = next.kind == gitcomet_core::domain::FileStatusKind::Conflicted;
        // Deferred: this runs while the window applies a new state.
        let pane = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = pane.update(cx, |pane, cx| {
                pane.open_status_file(
                    advance.repo_id,
                    advance.section,
                    advance.area,
                    advance.next,
                    is_conflicted,
                    cx,
                );
            });
        });
    }
}
