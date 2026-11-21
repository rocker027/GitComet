use super::*;
use crate::localization::Language;
use crate::view::panels::tests::{app_state_with_repo, opening_repo_state};
use crate::view::test_support::{push_test_state, redraw};
use gitcomet_core::domain::{Branch, CommitId, Worktree};
use gitcomet_core::worktree_finish::{
    AgentDispatchAvailability, FinishOperation, FinishSnapshot, InstalledAgent, IntegrationPreview,
    TreeId,
};
use gitcomet_state::model::{
    AgentDispatchState, CleanupOutcome, DriftKind, FinishMode, FinishSource, FinishTarget,
    ItemOutcome, ItemPhase, Loadable, RepoId, RepoState, RunPhase, TargetHost, WorktreeFinishRun,
    WorktreeFinishRunId,
};
use std::path::PathBuf;

pub(super) const MAIN_WT: &str = "/tmp/ws-finish/main";
pub(super) const FEAT_A_WT: &str = "/tmp/ws-finish/wt-a";
const FEAT_B_WT: &str = "/tmp/ws-finish/wt-b";

fn wt(path: &str, branch: Option<&str>) -> Worktree {
    Worktree {
        bare: false,
        path: PathBuf::from(path),
        head: Some(CommitId(Arc::from(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ))),
        branch: branch.map(str::to_string),
        detached: branch.is_none(),
    }
}

fn branch(name: &str) -> Branch {
    Branch {
        name: name.to_string(),
        target: CommitId(Arc::from("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")),
        upstream: None,
        divergence: None,
    }
}

pub(super) fn repo_with_worktrees(repo_id: RepoId) -> RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new(MAIN_WT));
    repo.common_dir = Some(Arc::from(Path::new(MAIN_WT).join(".git")));
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main"),
        branch("feat/a"),
        branch("feat/b"),
    ]));
    repo.worktrees = Loadable::Ready(Arc::new(vec![
        wt(MAIN_WT, Some("main")),
        wt(FEAT_A_WT, Some("feat/a")),
        wt(FEAT_B_WT, Some("feat/b")),
    ]));
    repo
}

pub(super) fn open_view(
    cx: &mut gpui::TestAppContext,
    state: Arc<AppState>,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            push_test_state(this, state, cx);
        });
        let _ = window.draw(app);
    });
    (view, cx)
}

fn open_popover(
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    kind: PopoverKind,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_centered(kind, window, cx);
            });
        });
    });
    redraw(cx);
}

fn sample_run(
    repo_id: RepoId,
    run_id: WorktreeFinishRunId,
    phase: RunPhase,
    item_phase: ItemPhase,
) -> WorktreeFinishRun {
    WorktreeFinishRun {
        id: run_id,
        origin_repo_id: repo_id,
        common_dir: PathBuf::from(MAIN_WT).join(".git"),
        mode: FinishMode::MergeInto,
        target: FinishTarget {
            branch: "main".to_string(),
            initial_head: Some(CommitId("1111111111111111111111111111111111111111".into())),
            locked_head: Some(CommitId("1111111111111111111111111111111111111111".into())),
            host: TargetHost::OpenTab {
                repo_id,
                path: PathBuf::from(MAIN_WT),
            },
            snapshot: None,
        },
        sources: vec![FinishSource {
            path: PathBuf::from(FEAT_A_WT),
            branch: "feat/a".to_string(),
            locked_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            initial_snapshot: Some(FinishSnapshot {
                worktree_exists: true,
                head: Some(CommitId("2222222222222222222222222222222222222222".into())),
                branch: Some("feat/a".to_string()),
                ..Default::default()
            }),
            initial_active_sessions: Vec::new(),
            squash: false,
            remove_after: false,
            squash_message: None,
            preview: Loadable::Ready(IntegrationPreview {
                ahead: 1,
                behind: 0,
                merged_tree: Some(TreeId::from("tree_ok")),
                target_tree: Some(TreeId::from("tree_target")),
                ..Default::default()
            }),
            phase: item_phase,
            result_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            expected_merged_tree: None,
            had_conflict: false,
            conflict_stops: 0,
            user_reviewed_resolution: false,
            dispatched_agent: false,
            agent_receipt_unknown: false,
            pending_retry_operation_id: None,
            conflict_baseline_entries: None,
            pending_squash_range: None,
            cleanup_disqualified: None,
            cleanup: CleanupOutcome::NotRequested,
            cleanup_notes: Vec::new(),
            last_git_output: None,
        }],
        phase,
        preview_blockers: Vec::new(),
        preview_warnings: Vec::new(),
        agent_dispatch: AgentDispatchState {
            availability: Some(AgentDispatchAvailability::Available {
                executable: PathBuf::from("/usr/local/bin/orca"),
            }),
            installed: Loadable::Ready(vec![InstalledAgent {
                id: "claude".to_string(),
                name: "Claude".to_string(),
                is_default: true,
            }]),
            last_error: None,
        },
        temp_worktree_kept_note: None,
        cleanup_agent_check_unverified: false,
        running_terminal_paths: Vec::new(),
        resume_from: 0,
        next_agent_request_id: 1,
        last_error: None,
    }
}

#[gpui::test]
fn t_u1_u2_u3_setup_defaults_unchecked_and_renders_mode_and_summary(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    let (view, cx) = open_view(cx, state);

    open_popover(
        &view,
        cx,
        PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::FinishSetup {
                preselected: vec![PathBuf::from(FEAT_A_WT)],
                target: Some("main".to_string()),
                mode: None,
            },
        ),
    );

    // T-U1: Both per-source checkboxes default to false.
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let draft = host
            .worktree_finish_setup
            .as_ref()
            .expect("setup draft initialized");
        assert_eq!(draft.target_branch, "main");
        assert_eq!(draft.mode, FinishMode::MergeInto);
        for row in &draft.rows {
            assert!(!row.squash, "Squash must default to unchecked");
            assert!(!row.remove_after, "Remove folder must default to unchecked");
        }
        assert!(
            draft.rows[1].selected,
            "preselected worktree should be checked"
        );
    });

    assert!(cx.debug_bounds("worktree_finish_mode_merge").is_some());
    assert!(cx.debug_bounds("worktree_finish_mode_rebase").is_some());
    assert!(cx.debug_bounds("worktree_finish_confirm_summary").is_some());
    assert!(cx.debug_bounds("worktree_finish_confirm_go").is_some());
}

#[gpui::test]
fn t_u6_queue_panel_and_action_bar_banner_show_state_specific_controls(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let run_id = WorktreeFinishRunId(77);
    let mut state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    Arc::make_mut(&mut state)
        .worktree_finish_runs
        .push(sample_run(
            repo_id,
            run_id,
            RunPhase::Paused { current: 0 },
            ItemPhase::PausedConflict {
                site: PathBuf::from(MAIN_WT),
                stops: 1,
                before_head: CommitId("1111111111111111111111111111111111111111".into()),
                snapshot: FinishSnapshot {
                    worktree_exists: true,
                    operation: FinishOperation::Merge,
                    unmerged: vec![PathBuf::from("conflict.rs")],
                    ..Default::default()
                },
            },
        ));
    let (view, cx) = open_view(cx, state);

    // Persistent Action Bar banner must be rendered while run is paused!
    assert!(
        cx.debug_bounds("worktree_finish_banner").is_some(),
        "action bar must display persistent Worktree Finish banner"
    );

    open_popover(
        &view,
        cx,
        PopoverKind::worktree(repo_id, WorktreePopoverKind::FinishQueue { run_id }),
    );
    assert!(cx.debug_bounds("worktree_finish_open_site").is_some());
    assert!(cx.debug_bounds("worktree_finish_recheck").is_some());
    assert!(cx.debug_bounds("worktree_finish_load_agents").is_some());
    assert!(
        cx.debug_bounds("worktree_finish_dispatch_agent_0")
            .is_some()
    );
    assert!(cx.debug_bounds("worktree_finish_skip").is_some());
    assert!(cx.debug_bounds("worktree_finish_abort_remaining").is_some());

    // Switch to AdoptCommit drift state -> Adopt & Rollback controls appear
    let mut adopt_state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    Arc::make_mut(&mut adopt_state)
        .worktree_finish_runs
        .push(sample_run(
            repo_id,
            run_id,
            RunPhase::Paused { current: 0 },
            ItemPhase::NeedsDecision {
                site: PathBuf::from(MAIN_WT),
                drift: DriftKind::AdoptCommit {
                    before_head: CommitId("1111111111111111111111111111111111111111".into()),
                    after_head: CommitId("3333333333333333333333333333333333333333".into()),
                    snapshot: FinishSnapshot::default(),
                },
            },
        ));
    cx.update(|window, app| {
        view.update(app, |this, cx| push_test_state(this, adopt_state, cx));
        let _ = window.draw(app);
    });
    redraw(cx);
    assert!(cx.debug_bounds("worktree_finish_adopt_commit").is_some());
    assert!(cx.debug_bounds("worktree_finish_rollback_commit").is_some());

    // Switch to Finished -> Dismiss button appears
    let mut done_state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    Arc::make_mut(&mut done_state)
        .worktree_finish_runs
        .push(sample_run(
            repo_id,
            run_id,
            RunPhase::Finished,
            ItemPhase::Done(ItemOutcome::NothingNew),
        ));
    cx.update(|window, app| {
        view.update(app, |this, cx| push_test_state(this, done_state, cx));
        let _ = window.draw(app);
    });
    redraw(cx);
    assert!(cx.debug_bounds("worktree_finish_dismiss_run").is_some());
}

#[test]
fn t_u4_u5_entries_include_worktree_row_section_palette_and_branch_drop() {
    let repo_id = RepoId(1);
    let section_en =
        super::super::context_menu::worktree_section::model(repo_id, Language::English);
    assert!(section_en.items.iter().any(|item| matches!(
        item,
        ContextMenuItem::Entry { label, disabled: false, .. } if label.as_ref() == "Finish worktrees…"
    )));
    let section_zh =
        super::super::context_menu::worktree_section::model(repo_id, Language::TraditionalChinese);
    assert!(section_zh.items.iter().any(|item| matches!(
        item,
        ContextMenuItem::Entry { label, disabled: false, .. } if label.as_ref() == "收尾工作樹…"
    )));

    let row_en = super::super::context_menu::worktree::model(
        repo_id,
        Path::new(FEAT_A_WT),
        Some("feat/a"),
        false,
        false,
        false,
        Language::English,
    );
    assert!(row_en.items.iter().any(|item| matches!(
        item,
        ContextMenuItem::Entry { label, disabled: false, .. } if label.as_ref() == "Finish into…"
    )));
    // F10: protected worktree disables Remove… and Finish into…
    let row_protected = super::super::context_menu::worktree::model(
        repo_id,
        Path::new(FEAT_A_WT),
        Some("feat/a"),
        false,
        false,
        true,
        Language::English,
    );
    assert!(row_protected.items.iter().any(|item| matches!(
        item,
        ContextMenuItem::Entry { label, disabled: true, .. } if label.as_ref() == "Remove…"
    )));
    assert!(row_protected.items.iter().any(|item| matches!(
        item,
        ContextMenuItem::Entry { label, disabled: true, .. } if label.as_ref() == "Finish into…"
    )));

    // T-U5: Branch drop with neither end checked out in current tab, but source has a worktree
    let source = BranchMenuTarget::local("feat/a");
    let target = BranchMenuTarget::local("feat/b");
    let facts = super::super::context_menu::branch_drop::BranchDropFacts {
        current: Some("main"),
        current_upstream: None,
        merge_allowed: true,
        history_rewrite_busy: false,
        annex_pushes: false,
        source_worktree: Some(PathBuf::from(FEAT_A_WT)),
        target_worktree: Some(PathBuf::from(FEAT_B_WT)),
    };
    let (items, _) = super::super::context_menu::branch_drop::drop_items(
        repo_id,
        &super::super::context_menu::branch_drop::BranchDropEnd {
            target: &source,
            reference: "feat/a".to_string(),
        },
        &super::super::context_menu::branch_drop::BranchDropEnd {
            target: &target,
            reference: "feat/b".to_string(),
        },
        &facts,
        Language::English,
    );
    let enabled_entries: Vec<_> = items
        .iter()
        .filter(|i| {
            matches!(
                i,
                ContextMenuItem::Entry {
                    disabled: false,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(
        enabled_entries.len(),
        2,
        "T-U5: cross-worktree Finish & Rebase entries must be enabled"
    );
}

#[gpui::test]
fn f6_f7_setup_selects_and_filters_over_12_target_branches_with_unchecked_options(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let mut repo = repo_with_worktrees(repo_id);
    let mut branches = vec![branch("main")];
    for idx in 1..=15 {
        branches.push(branch(&format!("release-{idx:02}")));
    }
    repo.branches = Loadable::Ready(Arc::new(branches));
    let state = app_state_with_repo(repo, repo_id);
    let (view, cx) = open_view(cx, state);

    open_popover(
        &view,
        cx,
        PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::FinishSetup {
                preselected: vec![PathBuf::from(FEAT_A_WT), PathBuf::from(FEAT_B_WT)],
                target: Some("main".to_string()),
                mode: None,
            },
        ),
    );

    // F7: multi-source setup keeps MergeInto default and both per-source checkboxes false.
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let draft = host
            .worktree_finish_setup
            .as_ref()
            .expect("setup draft initialized");
        assert_eq!(draft.mode, FinishMode::MergeInto);
        assert_eq!(draft.selected_requests().len(), 2);
        for req in draft.selected_requests() {
            assert!(!req.squash, "F7: squash must default to false");
            assert!(!req.remove_after, "F7: remove_after must default to false");
        }
    });

    // F6: 16th branch (index 15, beyond the old 12 limit) must be rendered & filterable!
    assert!(
        cx.debug_bounds("worktree_finish_target_filter").is_some(),
        "F6: target branch filter input must be rendered"
    );
    assert!(
        cx.debug_bounds("worktree_finish_target_15").is_some(),
        "F6: branch #16 (`release-15`) must be rendered without 12-item truncation"
    );

    // Filtering narrows the list live via PopoverHost observer (no manual cx.notify on PopoverHost!)
    // so `release-15` remains while `release-01` is hidden.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.worktree_finish_target_filter_input
                    .update(cx, |input, cx| {
                        input.set_text("release-15".to_string(), cx);
                    });
            });
        });
        let _ = window.draw(app);
    });
    redraw(cx);
    assert!(cx.debug_bounds("worktree_finish_target_15").is_some());
    assert!(cx.debug_bounds("worktree_finish_target_1").is_none());

    // F7: opening batch FinishSetup without preselected sources also keeps Squash and Remove unchecked.
    open_popover(
        &view,
        cx,
        PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::FinishSetup {
                preselected: Vec::new(),
                target: None,
                mode: None,
            },
        ),
    );
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let draft = host
            .worktree_finish_setup
            .as_ref()
            .expect("batch setup draft initialized");
        assert_eq!(draft.mode, FinishMode::MergeInto);
        assert!(
            draft
                .rows
                .iter()
                .all(|row| !row.squash && !row.remove_after)
        );
    });
}

#[gpui::test]
fn f5_f8_queue_ui_hides_dispatch_while_launching_and_shows_reconfirm_after_refresh(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let run_id = WorktreeFinishRunId(88);

    // F5: while AgentLaunching is in flight, dispatch buttons must NOT be rendered.
    let mut launching_state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    Arc::make_mut(&mut launching_state)
        .worktree_finish_runs
        .push(sample_run(
            repo_id,
            run_id,
            RunPhase::Paused { current: 0 },
            ItemPhase::AgentLaunching {
                site: PathBuf::from(MAIN_WT),
                stops: 1,
                before_head: CommitId("1111111111111111111111111111111111111111".into()),
                agent_id: "claude".to_string(),
                request_id: 1,
                started_at: std::time::SystemTime::UNIX_EPOCH,
                snapshot: FinishSnapshot::default(),
            },
        ));
    let (view, cx) = open_view(cx, launching_state);
    open_popover(
        &view,
        cx,
        PopoverKind::worktree(repo_id, WorktreePopoverKind::FinishQueue { run_id }),
    );
    assert!(
        cx.debug_bounds("worktree_finish_agent_launching").is_some(),
        "F5: launching indicator must be rendered"
    );
    assert!(
        cx.debug_bounds("worktree_finish_dispatch_agent_0")
            .is_none(),
        "F5: dispatch button must not be available while launch is in flight"
    );

    // F8: after RefreshPreview finishes, Queue panel shows explicit re-confirm button.
    let mut reconfirm_state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    Arc::make_mut(&mut reconfirm_state)
        .worktree_finish_runs
        .push(sample_run(
            repo_id,
            run_id,
            RunPhase::AwaitingConfirm { fingerprint: 9999 },
            ItemPhase::Queued,
        ));
    cx.update(|window, app| {
        view.update(app, |this, cx| push_test_state(this, reconfirm_state, cx));
        let _ = window.draw(app);
    });
    redraw(cx);
    assert!(
        cx.debug_bounds("worktree_finish_reconfirm_go").is_some(),
        "F8: Queue panel must render explicit second-confirmation button after RefreshPreview"
    );
}

#[gpui::test]
fn f10_active_finish_run_disables_conflicting_branch_and_commit_menu_entries(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let view_id = RepoId(2);
    let indep_id = RepoId(3);
    let run_id = WorktreeFinishRunId(99);
    let mut state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    let mut view_repo = opening_repo_state(view_id, Path::new("/tmp/ws-finish/wt-view"));
    view_repo.common_dir = Some(Arc::from(Path::new(MAIN_WT).join(".git")));
    view_repo.head_branch = Loadable::Ready("view".to_string());
    view_repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main"),
        branch("feat/a"),
        branch("view"),
        branch("unrelated"),
    ]));
    let mut indep_repo = opening_repo_state(indep_id, Path::new("/tmp/ws-independent/main"));
    indep_repo.common_dir = Some(Arc::from(Path::new("/tmp/ws-independent/main/.git")));
    indep_repo.head_branch = Loadable::Ready("dev".to_string());
    indep_repo.branches = Loadable::Ready(Arc::new(vec![branch("main"), branch("dev")]));
    {
        let s = Arc::make_mut(&mut state);
        s.repos.push(view_repo);
        s.repos.push(indep_repo);
        s.worktree_finish_runs.push(sample_run(
            repo_id,
            run_id,
            RunPhase::Paused { current: 0 },
            ItemPhase::PausedConflict {
                site: PathBuf::from(MAIN_WT),
                stops: 1,
                before_head: CommitId("1111111111111111111111111111111111111111".into()),
                snapshot: FinishSnapshot::default(),
            },
        ));
    }
    let (view, cx) = open_view(cx, state);

    cx.update(|_window, app| {
        let host_entity = view.read(app).popover_host.clone();
        host_entity.update(app, |host, cx| {
            let branch_menu = host
                .context_menu_model(
                    &PopoverKind::BranchMenu {
                        repo_id,
                        target: BranchMenuTarget::local("feat/a"),
                    },
                    cx,
                )
                .expect("branch context menu model");
            // Opening the other worktree is navigation, so it remains
            // available while Git mutations are blocked on this checkout.
            assert!(matches!(
                &branch_menu.items[branch_menu.first_selectable().unwrap()],
                ContextMenuItem::Entry { label, action, .. }
                    if label == "Open worktree" && matches!(
                        action.as_ref(),
                        ContextMenuAction::OpenRepo { path } if path == Path::new(FEAT_A_WT)
                    )
            ));
            assert!(!branch_menu.items.iter().any(|item| matches!(
                item,
                ContextMenuItem::Entry { action, .. } if matches!(
                    action.as_ref(), ContextMenuAction::CheckoutBranch { .. }
                )
            )));
            for required_disabled in [
                "Merge feat/a into main",
                "Squash feat/a into main",
                "Rename branch",
                "Delete branch",
            ] {
                assert!(
                    branch_menu.items.iter().any(|item| matches!(
                        item,
                        ContextMenuItem::Entry { label, disabled: true, .. }
                            if label.starts_with(required_disabled)
                    )),
                    "F10: branch menu entry '{required_disabled}' must be disabled during active finish run"
                );
            }

            // Third linked worktree sharing common_dir: protected `main` disables Rename/Delete/Checkout,
            // while unrelated branch remains enabled.
            let third_protected_menu = host
                .context_menu_model(
                    &PopoverKind::BranchMenu {
                        repo_id: view_id,
                        target: BranchMenuTarget::local("main"),
                    },
                    cx,
                )
                .expect("third tab protected branch menu");
            for required_disabled in ["Checkout", "Rename branch", "Delete branch"] {
                assert!(
                    third_protected_menu.items.iter().any(|item| matches!(
                        item,
                        ContextMenuItem::Entry { label, disabled: true, .. }
                            if label.starts_with(required_disabled)
                    )),
                    "F10: third tab must disable '{required_disabled}' on protected branch 'main'"
                );
            }
            let third_unrelated_menu = host
                .context_menu_model(
                    &PopoverKind::BranchMenu {
                        repo_id: view_id,
                        target: BranchMenuTarget::local("unrelated"),
                    },
                    cx,
                )
                .expect("third tab unrelated branch menu");
            for required_enabled in ["Checkout", "Rename branch", "Delete branch"] {
                assert!(
                    third_unrelated_menu.items.iter().any(|item| matches!(
                        item,
                        ContextMenuItem::Entry { label, disabled: false, .. }
                            if label.starts_with(required_enabled)
                    )),
                    "F10: third tab must keep '{required_enabled}' enabled on unrelated branch"
                );
            }

            // Independent repo with different common_dir must keep its own `main` enabled!
            let indep_menu = host
                .context_menu_model(
                    &PopoverKind::BranchMenu {
                        repo_id: indep_id,
                        target: BranchMenuTarget::local("main"),
                    },
                    cx,
                )
                .expect("independent repo branch menu");
            for required_enabled in ["Checkout", "Rename branch", "Delete branch"] {
                assert!(
                    indep_menu.items.iter().any(|item| matches!(
                        item,
                        ContextMenuItem::Entry { label, disabled: false, .. }
                            if label.starts_with(required_enabled)
                    )),
                    "F10: independent repo must keep '{required_enabled}' enabled on its own 'main'"
                );
            }

            let pull_menu = host
                .context_menu_model(&PopoverKind::PullPicker, cx)
                .expect("pull picker menu model");
            for required_disabled in [
                "Pull (default)",
                "Pull (rebase)",
                "Prune merged branches",
            ] {
                assert!(
                    pull_menu.items.iter().any(|item| matches!(
                        item,
                        ContextMenuItem::Entry { label, disabled: true, .. }
                            if label.as_ref() == required_disabled
                    )),
                    "F10: pull menu entry '{required_disabled}' must be disabled during active finish run"
                );
            }
        });
    });
}

#[test]
fn worktree_finish_merge_execution_location_waits_for_a_resolved_preview() {
    use super::super::worktree_finish_setup::execution_location_summary;

    let language = Language::TraditionalChinese;
    let mut run = sample_run(
        RepoId(1),
        WorktreeFinishRunId(301),
        RunPhase::Previewing,
        ItemPhase::Queued,
    );
    run.target.host = TargetHost::NotNeeded;
    assert_eq!(
        execution_location_summary(language, FinishMode::MergeInto, true, Some(&run)),
        "正在確認執行地點…"
    );
    assert_eq!(
        execution_location_summary(language, FinishMode::MergeInto, true, None),
        "正在確認執行地點…"
    );
    assert!(
        execution_location_summary(language, FinishMode::MergeInto, false, None)
            .contains("請選取來源工作樹")
    );
    assert!(
        execution_location_summary(language, FinishMode::RebaseOnto, true, Some(&run))
            .contains("目標分支不會移動")
    );

    run.phase = RunPhase::AwaitingConfirm { fingerprint: 31 };
    run.preview_blockers.push("Preview failed".to_string());
    assert_eq!(
        execution_location_summary(language, FinishMode::MergeInto, true, Some(&run)),
        "無法確認執行地點，請查看預覽錯誤。"
    );
    run.target.host = TargetHost::OpenTab {
        repo_id: RepoId(1),
        path: PathBuf::from(MAIN_WT),
    };
    assert_eq!(
        execution_location_summary(language, FinishMode::MergeInto, true, Some(&run)),
        format!("執行地點：{MAIN_WT}（已開分頁）")
    );
    assert!(
        !run.can_confirm(),
        "preview errors must retain their confirmation blocker"
    );
}

#[gpui::test]
fn worktree_finish_conflict_paths_expand_and_clear_when_the_draft_changes(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    let (view, cx) = open_view(cx, state);
    open_popover(
        &view,
        cx,
        PopoverKind::worktree(
            repo_id,
            WorktreePopoverKind::FinishSetup {
                preselected: vec![PathBuf::from(FEAT_A_WT)],
                target: Some("main".to_string()),
                mode: Some(FinishMode::MergeInto),
            },
        ),
    );
    let mut state = app_state_with_repo(repo_with_worktrees(repo_id), repo_id);
    let mut run = sample_run(
        repo_id,
        WorktreeFinishRunId(302),
        RunPhase::AwaitingConfirm { fingerprint: 32 },
        ItemPhase::Queued,
    );
    let Loadable::Ready(preview) = &mut run.sources[0].preview else {
        unreachable!();
    };
    preview.conflict_paths = vec![PathBuf::from("src/main.rs"), PathBuf::from("README.md")];
    Arc::make_mut(&mut state).worktree_finish_runs.push(run);
    cx.update(|window, app| {
        view.update(app, |this, cx| push_test_state(this, state, cx));
        let _ = window.draw(app);
    });
    redraw(cx);
    assert!(
        cx.debug_bounds("worktree_finish_conflicts_toggle_1")
            .is_some()
    );
    assert!(
        cx.debug_bounds("worktree_finish_conflict_path_1_0")
            .is_none()
    );

    let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let center = cx
            .debug_bounds(selector)
            .expect("visible conflict control")
            .center();
        cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.run_until_parked();
        redraw(cx);
    };
    click(cx, "worktree_finish_conflicts_toggle_1");
    assert!(
        cx.debug_bounds("worktree_finish_conflict_path_1_0")
            .is_some()
    );
    assert!(
        cx.debug_bounds("worktree_finish_conflict_path_1_1")
            .is_some()
    );
    click(cx, "worktree_finish_conflicts_toggle_1");
    assert!(
        cx.debug_bounds("worktree_finish_conflict_path_1_0")
            .is_none()
    );
    click(cx, "worktree_finish_conflicts_toggle_1");
    click(cx, "worktree_finish_mode_rebase");
    assert!(
        cx.debug_bounds("worktree_finish_conflict_path_1_0")
            .is_none()
    );
    cx.update(|_window, app| {
        let draft = view
            .read(app)
            .popover_host
            .read(app)
            .worktree_finish_setup
            .as_ref()
            .unwrap();
        assert!(draft.expanded_conflict_paths.is_empty());
        assert_eq!(draft.mode, FinishMode::RebaseOnto);
    });
}
