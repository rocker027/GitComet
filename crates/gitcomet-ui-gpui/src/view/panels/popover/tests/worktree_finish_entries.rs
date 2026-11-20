//! B2: every entry point of the Finish setup dialog must open without reading the root view.
//! The command palette (and any other root-side entry) opens the dialog while the root view is
//! being updated, where a root read is a GPUI double lease panic.

use super::branch::wait_until;
use super::worktree_finish::{FEAT_A_WT, MAIN_WT, open_view, repo_with_worktrees};
use super::*;
use crate::view::panels::tests::app_state_with_repo;
use crate::view::test_support::redraw;
use gitcomet_state::model::RepoId;
use std::path::PathBuf;

fn finish_setup_selection(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
) -> Vec<PathBuf> {
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        assert!(
            matches!(
                host.popover_kind_for_tests(),
                Some(PopoverKind::Repo {
                    repo_id: open_repo_id,
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::FinishSetup { .. }),
                }) if open_repo_id == repo_id
            ),
            "the Finish setup dialog must be the open popover"
        );
        host.worktree_finish_setup
            .as_ref()
            .expect("setup draft initialized")
            .rows
            .iter()
            .filter(|row| row.selected)
            .map(|row| row.path.clone())
            .collect()
    })
}

#[gpui::test]
fn b2_command_palette_entry_opens_finish_setup_inside_root_view_update(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    let (view, cx) = open_view(cx, state);

    // Same call chain as the palette: `CommandPalette` updates the root view, which runs the
    // command and opens the dialog through the popover host.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("finish-worktrees", Some(window), cx);
        });
    });
    redraw(cx);

    assert_eq!(
        finish_setup_selection(&view, cx, repo_id),
        Vec::<PathBuf>::new(),
        "the palette entry lists every worktree without preselecting one"
    );
    assert!(cx.debug_bounds("worktree_finish_confirm_go").is_some());
}

#[gpui::test]
fn action_bar_finish_button_opens_finish_setup_without_preselection(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    let (view, cx) = open_view(cx, state);
    redraw(cx);

    let center = cx
        .debug_bounds("finish_worktrees")
        .expect("the action bar's Finish button")
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    redraw(cx);

    assert_eq!(
        finish_setup_selection(&view, cx, repo_id),
        Vec::<PathBuf>::new(),
        "the toolbar entry lists every worktree without preselecting one"
    );
    assert!(cx.debug_bounds("worktree_finish_confirm_go").is_some());
}

#[gpui::test]
fn b2_context_menu_entry_opens_finish_setup_with_its_worktree_preselected(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    let (view, cx) = open_view(cx, state);
    let host = cx.update(|_window, app| view.read(app).popover_host.clone());

    // The worktree row menu activates its entry inside the popover host, outside any root
    // view update.
    cx.update(|window, app| {
        host.update(app, |host, cx| {
            host.context_menu_activate_action(
                ContextMenuAction::OpenPopover {
                    kind: PopoverKind::worktree(
                        repo_id,
                        WorktreePopoverKind::FinishSetup {
                            preselected: vec![PathBuf::from(FEAT_A_WT)],
                            target: None,
                            mode: None,
                        },
                    ),
                },
                window,
                cx,
            );
        });
    });
    redraw(cx);

    assert_eq!(
        finish_setup_selection(&view, cx, repo_id),
        vec![PathBuf::from(FEAT_A_WT)]
    );
    assert!(cx.debug_bounds("worktree_finish_confirm_go").is_some());
}

/// A root-side entry with a preselected source previews immediately, so it needs the
/// running-terminal paths while the root view is still being updated.
#[gpui::test]
fn b2_root_side_entry_previews_with_live_terminal_paths_inside_root_view_update(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    let (view, cx) = open_view(cx, Arc::clone(&state));
    let store = cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            // The store reduces the preview only for a repository it knows.
            this.store.replace_snapshot_for_test(state);
            // What the root view reports whenever an embedded terminal opens or closes.
            this.store
                .set_running_terminal_paths(vec![PathBuf::from(MAIN_WT)]);
            Arc::clone(&this.store)
        })
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.open_popover_centered(
                PopoverKind::worktree(
                    repo_id,
                    WorktreePopoverKind::FinishSetup {
                        preselected: vec![PathBuf::from(FEAT_A_WT)],
                        target: Some("main".to_string()),
                        mode: None,
                    },
                ),
                window,
                cx,
            );
        });
    });
    redraw(cx);

    assert_eq!(
        finish_setup_selection(&view, cx, repo_id),
        vec![PathBuf::from(FEAT_A_WT)]
    );
    wait_until("the preview to carry the live terminal paths", || {
        store.snapshot().worktree_finish_runs.iter().any(|run| {
            run.origin_repo_id == repo_id
                && run.running_terminal_paths == vec![PathBuf::from(MAIN_WT)]
        })
    });
}
