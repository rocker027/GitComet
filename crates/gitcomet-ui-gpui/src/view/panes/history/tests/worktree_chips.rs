use super::*;
use crate::view::caches::history_worktree_chips_by_head;
use gitcomet_core::domain::{Worktree, WorktreeDirtySummary};

fn worktree(path: impl Into<PathBuf>, head: Option<&str>, branch: Option<&str>) -> Worktree {
    Worktree {
        bare: false,
        path: path.into(),
        head: head.map(|head| CommitId(head.into())),
        branch: branch.map(str::to_owned),
        detached: false,
    }
}

#[test]
fn other_worktrees_are_grouped_by_the_commit_they_have_checked_out() {
    let chips = history_worktree_chips_by_head(
        &[
            worktree("/wt/own", Some("c0"), Some("main")),
            worktree("/wt/bare", None, None),
            worktree("/wt/a", Some("c1"), Some("side")),
            worktree("/wt/b", Some("c1"), None),
        ],
        Path::new("/wt/own"),
    );

    assert!(
        !chips.contains_key(&CommitId("c0".into())),
        "this worktree's HEAD is the HEAD row, not a chip"
    );
    let on_c1: Vec<_> = chips[&CommitId("c1".into())]
        .iter()
        .map(|chip| (chip.path.clone(), chip.label.to_string()))
        .collect();
    assert_eq!(
        on_c1,
        vec![
            (PathBuf::from("/wt/a"), "side · a".to_string()),
            (PathBuf::from("/wt/b"), "b".to_string()),
        ]
    );
    assert_eq!(chips.len(), 1, "a worktree with no HEAD gets no chip");
}

struct Mounted<'a> {
    cx: &'a mut gpui::VisualTestContext,
    view: Entity<GitCometView>,
    history: Entity<HistoryView>,
    store: AppStore,
    tip_row: usize,
    base_row: usize,
}

fn worktree_path(folder: &str) -> PathBuf {
    // `/tmp/...` has no drive prefix on Windows, so opening it adds one.
    // Use absolute fixture paths that stay the same when opened.
    std::env::temp_dir()
        .join("history-worktree-chips")
        .join(folder)
}

/// `tip` on `base`, opened at `main` on `tip`; `others` are
/// `(folder, branch)` worktrees on `base`, and `dirty` the folders with changes.
fn mount<'a>(
    cx: &'a mut gpui::TestAppContext,
    others: &[(&str, &str)],
    dirty: &[&str],
) -> Mounted<'a> {
    mount_with_repo(cx, others, dirty, |_| {})
}

fn mount_with_repo<'a>(
    cx: &'a mut gpui::TestAppContext,
    others: &[(&str, &str)],
    dirty: &[&str],
    configure: impl FnOnce(&mut RepoState),
) -> Mounted<'a> {
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let page = Arc::new(log_page(
        vec![commit("tip", &["base"], "tip"), commit("base", &[], "base")],
        None,
    ));
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: worktree_path("main"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    repo.branches_rev = 1;
    repo.log = Loadable::Ready(Arc::clone(&page));
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    let mut worktrees = vec![worktree(worktree_path("main"), Some("tip"), Some("main"))];
    worktrees.extend(
        others
            .iter()
            .map(|(folder, branch)| worktree(worktree_path(folder), Some("base"), Some(branch))),
    );
    repo.worktrees = Loadable::Ready(Arc::new(worktrees));
    repo.worktrees_rev = 1;
    repo.worktree_dirty = Loadable::Ready(Arc::new(
        dirty
            .iter()
            .map(|folder| WorktreeDirtySummary {
                path: worktree_path(folder),
                head: Some(CommitId("base".into())),
                modified: 1,
                ..Default::default()
            })
            .collect(),
    ));
    repo.worktree_dirty_rev = 1;
    configure(&mut repo);
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    });

    ensure_history_cache_for_tests(cx, &view, state);
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    wait_until(cx, "history rows", |cx| {
        cx.update(|_, app| {
            history
                .read(app)
                .history_cache
                .as_ref()
                .is_some_and(|cache| cache.base.row_vms.len() == 2)
        })
    });
    let (tip_row, base_row) = cx.update(|_, app| {
        history.update(app, |history, _| {
            let plan = history.ensure_history_list_plan();
            (plan.list_ix_for_visible(0), plan.list_ix_for_visible(1))
        })
    });
    for _ in 0..2 {
        crate::view::test_support::redraw(cx);
    }
    Mounted {
        cx,
        view,
        history,
        store: store_for_assert,
        tip_row,
        base_row,
    }
}

fn bounds_of(cx: &mut gpui::VisualTestContext, selector: String) -> Option<Bounds<Pixels>> {
    cx.debug_bounds(Box::leak(selector.into_boxed_str()))
}

fn wait_for_open(cx: &mut gpui::VisualTestContext, store: &AppStore, folder: &str) {
    let path = worktree_path(folder);
    wait_until(cx, &format!("{folder} to open"), |_| {
        store
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == path)
    });
}

fn assert_shared_tip_branch_double_click_opens_linked_worktree(
    cx: &mut gpui::TestAppContext,
    mode: HistoryBranchNamesMode,
) {
    let Mounted {
        cx,
        view,
        history,
        store,
        base_row,
        ..
    } = mount_with_repo(
        cx,
        &[("alias-linked", "alias"), ("target-linked", "side")],
        &[],
        |repo| {
            repo.open = Loadable::Ready(());
            repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
            repo.branches = Loadable::Ready(Arc::new(vec![
                branch("main", "tip"),
                branch("alias", "base"),
                branch("side", "base"),
            ]));
            repo.history_state.selected_commit = Some(CommitId("tip".into()));
        },
    );
    // Allocate the original repo's ID through the real store before installing
    // the fixture snapshot, so opening the linked path gets a distinct ID.
    let fixture_state = cx.update(|_, app| history.read(app).state.clone());
    store.dispatch(Msg::OpenRepo(worktree_path("main")));
    wait_for_open(cx, &store, "main");
    store.replace_snapshot_for_test(fixture_state.clone());
    ensure_history_cache_for_tests(cx, &view, fixture_state);
    cx.simulate_resize(size(px(2000.0), px(900.0)));
    cx.update(|window, app| {
        history.update(app, |history, cx| {
            history.history_branch_names = mode;
            history.history_col_branch = px(320.0);
            history.history_col_graph = px(100.0);
            history.set_history_column_preferences(true, false, false, false, cx);
            window.activate();
            window.focus(&history.history_panel_focus_handle, cx);
            cx.notify();
        });
    });
    crate::view::test_support::redraw(cx);
    let row = bounds_of(cx, format!("history_row_{base_row}")).expect("the shared-tip row");
    let branch_bounds = |name| {
        HistoryView::painted_ref_bounds_for_test(
            RepoId(1),
            "base",
            mode,
            &HistoryMenuRef::Branch(BranchMenuTarget::local(name)),
        )
        .expect("both local branch labels must be visible in the actual canvas paint")
    };
    let alias = branch_bounds("alias");
    let side = branch_bounds("side");
    assert_eq!(alias.intersect(&row), alias);
    assert_eq!(side.intersect(&row), side);
    assert!(alias.right() <= side.left() || side.right() <= alias.left());
    let before = store.snapshot();
    let original = before
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == worktree_path("main"))
        .unwrap();
    let original_id = original.id;
    let original_ops_rev = original.ops_rev;
    cx.simulate_mouse_move(side.center(), None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: side.center(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: side.center(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        ..Default::default()
    });
    cx.run_until_parked();
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(3))
        .unwrap();
    assert_eq!(
        store.snapshot().repos[0].history_state.selected_commit,
        Some(CommitId("base".into()))
    );
    cx.simulate_event(gpui::MouseDownEvent {
        position: side.center(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: side.center(),
        button: gpui::MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    wait_for_open(cx, &store, "target-linked");
    let after = store.snapshot();
    let opened = after
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == worktree_path("target-linked"))
        .unwrap();
    assert_ne!(opened.id, original_id);
    assert_eq!(after.active_repo, Some(opened.id));
    assert!(
        !after
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == worktree_path("alias-linked")),
        "the other branch at the same SHA must not determine which worktree opens"
    );
    let original = after
        .repos
        .iter()
        .find(|repo| repo.id == original_id)
        .unwrap();
    assert_eq!(original.spec.workdir, worktree_path("main"));
    assert_eq!(original.head_branch, Loadable::Ready("main".into()));
    assert_eq!(
        original.ops_rev, original_ops_rev,
        "CheckoutBranch enters a local action and advances ops_rev, even if it later fails"
    );
    assert_eq!(original.local_actions_in_flight, 0);
    assert_eq!(
        original.history_state.selected_commit,
        Some(CommitId("base".into()))
    );
}

#[gpui::test]
fn separate_graph_branch_double_click_opens_exact_linked_worktree_at_shared_tip(
    cx: &mut gpui::TestAppContext,
) {
    assert_shared_tip_branch_double_click_opens_linked_worktree(
        cx,
        HistoryBranchNamesMode::SeparateColumn,
    );
}

#[gpui::test]
fn inline_graph_branch_double_click_opens_exact_linked_worktree_at_shared_tip(
    cx: &mut gpui::TestAppContext,
) {
    assert_shared_tip_branch_double_click_opens_linked_worktree(cx, HistoryBranchNamesMode::Inline);
}

#[gpui::test]
fn graph_linked_branch_menu_opens_exact_worktree_first_without_checkout(
    cx: &mut gpui::TestAppContext,
) {
    use crate::view::panels::ContextMenuAction;
    cx.update(|app| app.set_global(crate::localization::Language::English));
    let Mounted {
        cx,
        view,
        history,
        store,
        ..
    } = mount_with_repo(
        cx,
        &[("alias-linked", "alias"), ("target-linked", "side")],
        &[],
        |repo| {
            repo.open = Loadable::Ready(());
            repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
            repo.branches = Loadable::Ready(Arc::new(vec![
                branch("main", "tip"),
                branch("alias", "base"),
                branch("side", "base"),
            ]));
            repo.history_state.selected_commit = Some(CommitId("tip".into()));
        },
    );
    let fixture_state = cx.update(|_, app| history.read(app).state.clone());
    store.dispatch(Msg::OpenRepo(worktree_path("main")));
    wait_for_open(cx, &store, "main");
    store.replace_snapshot_for_test(fixture_state.clone());
    ensure_history_cache_for_tests(cx, &view, fixture_state);
    cx.simulate_resize(size(px(2000.0), px(900.0)));
    cx.update(|_, app| {
        history.update(app, |history, cx| {
            history.history_branch_names = HistoryBranchNamesMode::SeparateColumn;
            history.history_col_branch = px(320.0);
            cx.notify();
        });
    });
    crate::view::test_support::redraw(cx);
    let bounds = |name: &str| {
        HistoryView::painted_ref_bounds_for_test(
            RepoId(1),
            "base",
            HistoryBranchNamesMode::SeparateColumn,
            &HistoryMenuRef::Branch(BranchMenuTarget::local(name)),
        )
        .expect("the shared-tip local branch label is painted")
    };
    let target = bounds("side").center();
    assert!(!bounds("alias").contains(&target));
    let before = store.snapshot();
    let original_ops = before.repos[0].ops_rev;
    cx.simulate_mouse_move(target, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(target, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(target, gpui::MouseButton::Right, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    let kind = PopoverKind::BranchMenu {
        repo_id: RepoId(1),
        target: BranchMenuTarget::local("side"),
    };
    let actions = cx.update(|_, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(kind.clone())
        );
        let host = view.read(app).popover_host.clone();
        host.update(app, |host, cx| {
            host.context_menu_actions_for_test(&kind, cx)
        })
    });
    let expected_path = worktree_path("target-linked");
    assert!(
        matches!(actions.first(), Some(ContextMenuAction::OpenRepo {path}) if path == &expected_path),
        "the first actionable row must open the precise linked worktree, not check out its branch"
    );
    assert_eq!(
        actions
            .iter()
            .filter(|action| matches!(action, ContextMenuAction::OpenRepo { .. }))
            .count(),
        1,
        "the Open worktree action appears exactly once"
    );
    assert!(
        !actions
            .iter()
            .any(|action| matches!(action, ContextMenuAction::CheckoutBranch { .. })),
        "a linked local branch must not offer CheckoutBranch"
    );
    assert!(cx.debug_bounds("context_menu_checkout").is_none());
    let open = cx
        .debug_bounds("context_menu_open_worktree")
        .expect("the actual Open worktree menu item");
    cx.simulate_mouse_move(open.center(), None, gpui::Modifiers::default());
    cx.simulate_click(open.center(), gpui::Modifiers::default());
    wait_for_open(cx, &store, "target-linked");
    let snapshot = store.snapshot();
    let opened = snapshot
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == expected_path)
        .unwrap();
    assert_eq!(snapshot.active_repo, Some(opened.id));
    assert_ne!(opened.id, RepoId(1));
    assert!(
        !snapshot
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == worktree_path("alias-linked"))
    );
    let original = snapshot
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(1))
        .unwrap();
    assert_eq!(original.spec.workdir, worktree_path("main"));
    assert_eq!(original.head_branch, Loadable::Ready("main".into()));
    assert_eq!(original.ops_rev, original_ops);
    assert_eq!(original.local_actions_in_flight, 0);
}

#[gpui::test]
fn graph_complete_double_click_checks_out_exact_unlinked_branch_in_real_git(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = gitcomet_core::test_support::git_fixture::git_command()
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--allow-empty",
        "-qm",
        "fixture",
    ]);
    git(&["branch", "a"]);
    git(&["branch", "b"]);
    let sha = git(&["rev-parse", "HEAD"]);
    let refs_before = git(&["show-ref", "--heads"]);
    let repository = gitcomet_git_gix::GixBackend.open(dir.path()).unwrap();
    let original_path = repository.spec().workdir.clone();
    let (view, cx, mut state, store) = refresh::mount(
        cx,
        Arc::new(log_page(vec![commit(&sha, &[], "fixture")], None)),
    );
    let repo = &mut state.repos[0];
    repo.spec = repository.spec().clone();
    repo.head_branch = Loadable::Ready("main".into());
    repo.head_branch_rev += 1;
    repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main", &sha),
        branch("a", &sha),
        branch("b", &sha),
    ]));
    repo.branches_rev += 1;
    repo.worktrees = Loadable::Ready(Arc::new(vec![worktree(
        original_path.clone(),
        Some(&sha),
        Some("main"),
    )]));
    repo.worktrees_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    store.insert_repo_for_test(RepoId(1), repository);
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    cx.simulate_resize(size(px(2000.0), px(900.0)));
    cx.update(|window, app| {
        let history = view.read(app).main_pane.read(app).history_view.clone();
        history.update(app, |history, cx| {
            history.history_branch_names = HistoryBranchNamesMode::SeparateColumn;
            history.history_col_branch = px(320.0);
            window.activate();
            window.focus(&history.history_panel_focus_handle, cx);
            cx.notify();
        });
    });
    crate::view::test_support::redraw(cx);
    let bounds = |name: &str| {
        HistoryView::painted_ref_bounds_for_test(
            RepoId(1),
            &sha,
            HistoryBranchNamesMode::SeparateColumn,
            &HistoryMenuRef::Branch(BranchMenuTarget::local(name)),
        )
        .expect("both unlinked branch labels are painted at the shared SHA")
    };
    let target = bounds("b").center();
    assert!(!bounds("a").contains(&target));
    cx.simulate_mouse_move(target, None, gpui::Modifiers::default());
    for click_count in [1, 2] {
        cx.simulate_event(gpui::MouseDownEvent {
            position: target,
            button: gpui::MouseButton::Left,
            click_count,
            ..Default::default()
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position: target,
            button: gpui::MouseButton::Left,
            click_count,
            ..Default::default()
        });
        cx.run_until_parked();
        if click_count == 1 {
            store
                .barrier_for_test()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            assert_eq!(
                store.snapshot().repos[0]
                    .history_state
                    .selected_commit
                    .as_ref()
                    .map(|id| id.as_ref()),
                Some(sha.as_str())
            );
            assert_eq!(git(&["symbolic-ref", "HEAD"]), "refs/heads/main");
        }
    }
    wait_until(
        cx,
        "the graph double click checks out b in the real repository",
        |_| git(&["symbolic-ref", "HEAD"]) == "refs/heads/b",
    );
    assert_eq!(git(&["rev-parse", "HEAD"]), sha);
    assert_eq!(git(&["show-ref", "--heads"]), refs_before);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.repos.len(), 1);
    assert_eq!(snapshot.repos[0].spec.workdir, original_path);
}

#[gpui::test]
fn graph_remote_double_click_tracks_exact_remote_and_rejects_mixed_ref_gestures(
    cx: &mut gpui::TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = gitcomet_core::test_support::git_fixture::git_command()
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    git(&["commit", "--allow-empty", "-qm", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    git(&["commit", "--allow-empty", "-qm", "remote tip"]);
    let tip = git(&["rev-parse", "HEAD"]);
    git(&["branch", "origin/topic"]);
    let remote_url = dir.path().join("local-upstream.git").display().to_string();
    // Same short name and SHA must still expose two exact remote targets.
    for (remote, remote_branch) in [("origin", "topic"), ("peer", "topic")] {
        git(&["remote", "add", remote, &remote_url]);
        git(&[
            "update-ref",
            &format!("refs/remotes/{remote}/{remote_branch}"),
            &tip,
        ]);
    }
    git(&["reset", "--hard", &base]);
    let refs_before = git(&["show-ref"]);
    let repository = gitcomet_git_gix::GixBackend.open(dir.path()).unwrap();
    let original_path = repository.spec().workdir.clone();
    let (view, cx, mut state, store) = refresh::mount(
        cx,
        Arc::new(log_page(
            vec![
                commit(&tip, &[&base], "remote tip"),
                commit(&base, &[], "base"),
            ],
            None,
        )),
    );
    let repo = &mut state.repos[0];
    repo.spec = repository.spec().clone();
    repo.head_branch = Loadable::Ready("main".into());
    repo.head_branch_rev += 1;
    repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main", &base),
        branch("origin/topic", &tip),
    ]));
    repo.branches_rev += 1;
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        remote_branch("origin", "topic", &tip),
        remote_branch("peer", "topic", &tip),
    ]));
    repo.remote_branches_rev += 1;
    repo.remotes = Loadable::Ready(Arc::new(
        ["origin", "peer"]
            .map(|name| gitcomet_core::domain::Remote {
                name: name.into(),
                url: Some(remote_url.clone()),
                default_branch: None,
            })
            .into(),
    ));
    repo.worktrees = Loadable::Ready(Arc::new(vec![worktree(
        original_path.clone(),
        Some(&base),
        Some("main"),
    )]));
    repo.worktrees_rev += 1;
    store.disable_repo_monitors_for_test();
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    store.insert_repo_for_test(RepoId(1), repository);
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    cx.simulate_resize(size(px(2400.0), px(900.0)));
    cx.update(|window, app| {
        let history = view.read(app).main_pane.read(app).history_view.clone();
        history.update(app, |history, cx| {
            history.history_branch_names = HistoryBranchNamesMode::SeparateColumn;
            history.history_col_branch = px(520.0);
            history.set_history_column_preferences(true, false, false, false, cx);
            window.activate();
            window.focus(&history.history_panel_focus_handle, cx);
            cx.notify();
        });
    });
    let bounds = |target: &HistoryMenuRef| {
        HistoryView::painted_ref_bounds_for_test(
            RepoId(1),
            &tip,
            HistoryBranchNamesMode::SeparateColumn,
            target,
        )
        .expect("local and both remote labels must be visible in the actual canvas paint")
    };
    let click_label = |cx: &mut gpui::VisualTestContext, target: &HistoryMenuRef, click_count| {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
        let position = bounds(target).center();
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.simulate_event(gpui::MouseDownEvent {
            position,
            button: gpui::MouseButton::Left,
            click_count,
            ..Default::default()
        });
        cx.simulate_event(gpui::MouseUpEvent {
            position,
            button: gpui::MouseButton::Left,
            click_count,
            ..Default::default()
        });
        cx.run_until_parked();
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        sync_view_with_store(cx, &view);
    };
    let local = HistoryMenuRef::Branch(BranchMenuTarget::local("origin/topic"));
    let origin = HistoryMenuRef::Branch(BranchMenuTarget::remote("origin", "topic"));
    let peer = HistoryMenuRef::Branch(BranchMenuTarget::remote("peer", "topic"));
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    for (left, right) in [(&local, &origin), (&origin, &peer), (&local, &peer)] {
        let a = bounds(left);
        let b = bounds(right);
        assert!(
            a.right() <= b.left() || b.right() <= a.left(),
            "the two typed refs must have distinct painted gesture targets"
        );
    }
    for target in [&origin, &peer] {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
        let position = bounds(target).center();
        cx.simulate_mouse_down(
            position,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            position,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        let HistoryMenuRef::Branch(branch_target) = target else {
            unreachable!()
        };
        cx.update(|_, app| {
            assert_eq!(
                crate::view::test_support::popover_kind(view.read(app), app),
                Some(PopoverKind::BranchMenu {
                    repo_id: RepoId(1),
                    target: branch_target.clone()
                }),
            )
        });
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        cx.update(|window, app| {
            assert!(crate::view::test_support::popover_kind(view.read(app), app).is_none());
            let history = view.read(app).main_pane.read(app).history_view.read(app);
            assert!(history.history_panel_focus_handle.is_focused(window));
        });
    }
    for (first, second) in [(&local, &origin), (&origin, &peer)] {
        click_label(cx, first, 1);
        click_label(cx, second, 2);
        wait_until(cx, "mixed gesture has no pending checkout", |_| {
            store.snapshot().repos[0].local_actions_in_flight == 0
        });
        assert_eq!(
            git(&["symbolic-ref", "HEAD"]),
            "refs/heads/main",
            "different ref identities must not combine into a double click"
        );
        assert_eq!(git(&["show-ref"]), refs_before);
        cx.update(|_, app| {
            assert!(crate::view::test_support::popover_kind(view.read(app), app).is_none())
        });
    }
    click_label(cx, &origin, 1);
    assert_eq!(
        git(&["symbolic-ref", "HEAD"]),
        "refs/heads/main",
        "one remote-label click only selects its commit"
    );
    click_label(cx, &origin, 2);
    wait_until(
        cx,
        "remote-label double click creates the tracking branch in real Git",
        |_| {
            git(&["symbolic-ref", "HEAD"]) == "refs/heads/topic"
                && store.snapshot().repos[0].local_actions_in_flight == 0
        },
    );
    assert_eq!(git(&["rev-parse", "HEAD"]), tip);
    assert_eq!(git(&["config", "branch.topic.remote"]), "origin");
    assert_eq!(git(&["config", "branch.topic.merge"]), "refs/heads/topic");
    assert_eq!(git(&["rev-parse", "refs/heads/main"]), base);
    for reference in [
        "refs/heads/origin/topic",
        "refs/remotes/origin/topic",
        "refs/remotes/peer/topic",
    ] {
        assert_eq!(git(&["rev-parse", reference]), tip);
    }
    assert_eq!(git(&["status", "--porcelain=v1"]), "");
    let snapshot = store.snapshot();
    assert_eq!(snapshot.repos.len(), 1);
    assert_eq!(snapshot.repos[0].spec.workdir, original_path);
}

#[gpui::test]
fn history_refs_hover_marks_only_the_local_ref_checked_out_in_a_worktree(
    cx: &mut gpui::TestAppContext,
) {
    let Mounted {
        cx, view, base_row, ..
    } = mount_with_repo(cx, &[("linked", "side")], &["linked"], |repo| {
        repo.branches = Loadable::Ready(Arc::new(vec![
            branch("main", "tip"),
            branch("alias", "base"),
            branch("side", "base"),
        ]));
        repo.remote_branches =
            Loadable::Ready(Arc::new(vec![remote_branch("origin", "side", "base")]));
        repo.remote_branches_rev = 1;
    });
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_history_branch_names(HistoryBranchNamesMode::SeparateColumn, cx);
        });
    });
    crate::view::test_support::redraw(cx);
    let row = bounds_of(cx, format!("history_row_{base_row}")).expect("the base commit's row");
    let refs_center = point(row.left() + px(24.0), row.center().y);
    cx.simulate_mouse_move(refs_center, None, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);

    for selector in [
        "history_refs_hover_item_local_branch_side",
        "history_refs_hover_item_local_branch_alias",
        "history_refs_hover_item_remote_branch_origin_side",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "{selector} should be listed"
        );
    }
    let badge = cx
        .debug_bounds("history_refs_hover_item_local_branch_side_worktree")
        .expect("the matching local ref names its worktree even when that worktree is dirty");
    assert!(
        cx.debug_bounds("history_refs_hover_item_local_branch_alias_worktree")
            .is_none()
    );
    assert!(
        cx.debug_bounds("history_refs_hover_item_remote_branch_origin_side_worktree")
            .is_none()
    );
    cx.simulate_mouse_move(badge.center(), None, gpui::Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    let tooltip = crate::view::test_support::tooltip_text(cx, &view)
        .expect("the worktree badge shows its full path");
    assert!(tooltip.contains(worktree_path("linked").to_string_lossy().as_ref()));
}

/// A clean worktree has no row of its own, so its chip sits on the commit it
/// has checked out. A dirty one keeps its chip on its changes row.
#[gpui::test]
fn a_clean_worktree_gets_a_chip_on_its_head_commit(cx: &mut gpui::TestAppContext) {
    let Mounted {
        cx,
        view,
        history,
        store,
        tip_row,
        base_row,
    } = mount(cx, &[("clean", "side"), ("dirty", "fix")], &["dirty"]);

    let row = bounds_of(cx, format!("history_row_{base_row}")).expect("the base commit's row");
    let chip = bounds_of(cx, format!("history_worktree_chip_{base_row}"))
        .expect("the clean worktree should have a chip on the commit it has checked out");
    assert!(
        row.contains(&chip.center()) && chip.center().x > row.center().x,
        "the chip belongs at the end of its commit's message, got {chip:?} in {row:?}"
    );
    assert!(
        bounds_of(cx, format!("history_worktree_more_{base_row}")).is_none(),
        "the dirty worktree's chip is on its changes row, so nothing is folded here"
    );
    assert!(
        bounds_of(cx, format!("history_worktree_chip_{tip_row}")).is_none(),
        "this worktree gets no chip"
    );

    // The message ends where the chip starts, so pointing at the chip is not
    // pointing at the message.
    let hover_card_at = |position: Point<Pixels>, cx: &mut gpui::VisualTestContext| {
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(800));
        crate::view::test_support::redraw(cx);
        let shown = cx.debug_bounds("commit_message_hover").is_some();
        cx.update(|_, app| view.update(app, |view, cx| view.dismiss_commit_message_hover(cx)));
        shown
    };
    assert!(
        !hover_card_at(chip.center(), cx),
        "the commit message card must not open over the chip"
    );
    assert!(
        hover_card_at(point(chip.left() - px(40.0), chip.center().y), cx),
        "the message beside the chip still opens its card"
    );

    cx.simulate_click(chip.center(), gpui::Modifiers::default());
    assert!(
        cx.update(|_, app| history.read(app).pending_history_selections.is_empty()),
        "clicking the chip must not also select the commit under it"
    );
    wait_for_open(cx, &store, "clean");

    // Control: the same row, clicked beside the chip, selects its commit.
    cx.simulate_click(
        point(chip.left() - px(40.0), chip.center().y),
        gpui::Modifiers::default(),
    );
    assert!(
        cx.update(|_, app| !history.read(app).pending_history_selections.is_empty()),
        "a click on the row itself should select the commit"
    );
}

/// A commit shows one chip and "+N"; the "+N" opens a menu of every clean
/// worktree there, so the folded ones stay reachable. The chips take at most
/// half the message cell.
#[gpui::test]
fn more_worktrees_on_a_commit_fold_into_a_menu(cx: &mut gpui::TestAppContext) {
    let Mounted {
        cx,
        view,
        store,
        base_row,
        ..
    } = mount(
        cx,
        &[
            ("a-clean-worktree-with-a-long-folder-name", "first"),
            ("another-clean-worktree-with-a-long-name", "second"),
            ("dirty", "fix"),
        ],
        &["dirty"],
    );
    cx.simulate_resize(gpui::size(px(1100.0), px(700.0)));
    crate::view::test_support::redraw(cx);

    let message = cx.debug_bounds("history_message_header_cell").unwrap();
    let chip = bounds_of(cx, format!("history_worktree_chip_{base_row}"))
        .expect("the first worktree's chip");
    let more = bounds_of(cx, format!("history_worktree_more_{base_row}"))
        .expect("the rest fold into a +N chip");
    assert!(
        chip.right() <= more.left() && more.right() <= message.right(),
        "+N follows the chip at the end of the message cell"
    );
    assert!(
        chip.left() >= message.center().x - px(8.0),
        "the chips take at most half the message cell: {chip:?} in {message:?}"
    );

    cx.simulate_click(more.center(), gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert_eq!(
        cx.update(|_, app| crate::view::test_support::popover_kind(view.read(app), app)),
        Some(PopoverKind::worktree(
            RepoId(1),
            WorktreePopoverKind::CommitMenu {
                commit_id: CommitId("base".into()),
            },
        ))
    );
    assert!(cx.debug_bounds("history_worktree_menu_0").is_some());
    assert!(
        cx.debug_bounds("history_worktree_menu_2").is_none(),
        "the dirty worktree is not on this row, so not in its menu"
    );
    let second = cx
        .debug_bounds("history_worktree_menu_1")
        .expect("the folded worktree is in the menu");
    cx.simulate_click(second.center(), gpui::Modifiers::default());
    wait_for_open(cx, &store, "another-clean-worktree-with-a-long-name");
}

fn assert_clean_worktree_chip_compacts_on_resize(cx: &mut gpui::TestAppContext, percent: u32) {
    cx.update(|app| {
        crate::appearance::pin_compact_for_test(app);
        crate::ui_scale::set_default(app, percent);
        app.set_global(crate::localization::Language::English);
    });
    let Mounted {
        cx,
        view,
        history,
        store,
        base_row,
        ..
    } = mount_with_repo(cx, &[("qa-b", "qa-b"), ("qa-c", "qa-c")], &[], |repo| {
        repo.history_state.selected_commit = Some(CommitId("tip".into()));
    });
    cx.update(|window, app| {
        crate::ui_scale::apply_to_window(window, percent);
        history.update(app, |history, cx| {
            history.ui_scale_percent = percent;
            history.set_history_column_preferences(false, false, false, false, cx);
            cx.notify();
        });
    });
    let scaled = |value: f32| px(value * percent as f32 / 100.0);
    let selector = format!("history_worktree_chip_{base_row}");
    let more_selector = format!("history_worktree_more_{base_row}");
    let resize = |cx: &mut gpui::VisualTestContext, width| {
        cx.simulate_resize(gpui::size(width, scaled(800.0)));
        crate::view::test_support::redraw(cx);
        crate::view::test_support::redraw(cx);
    };
    resize(cx, scaled(1800.0));
    let wide_message = cx.debug_bounds("history_message_header_cell").unwrap();
    let wide_chip = bounds_of(cx, selector.clone()).expect("wide clean worktree chip");
    assert!(
        wide_message.size.width > scaled(400.0),
        "wide fixture: {wide_message:?}"
    );

    // Choose by the measured message cell, rather than assuming how much of
    // the window the sidebar or the current column layout consumes. Preserve
    // 220 design px for the summary and leave under 96 for the chips.
    let mut narrow = None;
    for width in (400..=1800).rev().step_by(10) {
        resize(cx, scaled(width as f32));
        let Some(message) = cx.debug_bounds("history_message_header_cell") else {
            continue;
        };
        if message.size.width >= scaled(280.0) && message.size.width < scaled(316.0) {
            narrow = Some((scaled(width as f32), message));
            break;
        }
    }
    let (narrow_width, message) =
        narrow.expect("fixture reaches a 280..316 design-px message cell");
    let chip = bounds_of(cx, selector.clone()).expect("compact chip keeps its selector");
    let more = bounds_of(cx, more_selector.clone()).expect("+N stays reachable when compact");
    assert!(
        chip.size.width >= scaled(20.0) && chip.size.width <= scaled(28.0),
        "compact icon target: {chip:?}"
    );
    assert!(
        wide_chip.size.width > chip.size.width + scaled(8.0),
        "wide label must expand: wide={wide_chip:?}, compact={chip:?}"
    );
    assert!(
        chip.left() >= message.left() + scaled(220.0),
        "chip must preserve summary space: {chip:?} in {message:?}"
    );
    assert!(
        chip.right() <= more.left() && more.right() <= message.right(),
        "both controls stay in the message cell"
    );
    println!(
        "scale={percent}% wide_message={:?} wide_chip={:?} narrow_message={:?} compact_chip={:?} summary_room={:?}",
        wide_message.size.width,
        wide_chip.size.width,
        message.size.width,
        chip.size.width,
        chip.left() - message.left(),
    );

    cx.simulate_mouse_move(chip.center(), None, gpui::Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    let tooltip = crate::view::test_support::tooltip_text(cx, &view).expect("compact chip tooltip");
    assert!(tooltip.contains("Open this worktree"), "{tooltip}");
    let full_path = worktree_path("qa-b").to_string_lossy().into_owned();
    assert!(
        tooltip.replace(&full_path, "").contains("qa-b"),
        "full label must be present in addition to its path: {tooltip}"
    );
    assert!(!tooltip.contains("qa-b · qa-b"), "{tooltip}");
    assert!(
        tooltip.contains(&full_path),
        "full worktree path: {tooltip}"
    );

    resize(cx, scaled(1800.0));
    let expanded = bounds_of(cx, selector.clone()).expect("same chip expands after resize");
    assert!(expanded.size.width > chip.size.width + scaled(8.0));
    resize(cx, narrow_width);
    let compact = bounds_of(cx, selector).expect("same chip compacts again");
    assert!((compact.size.width - chip.size.width).abs() <= px(1.0));
    assert!(
        bounds_of(cx, more_selector).is_some(),
        "+N survives both resizes"
    );
    cx.simulate_mouse_move(compact.center(), None, gpui::Modifiers::default());
    cx.simulate_click(compact.center(), gpui::Modifiers::default());
    cx.update(|_, app| {
        let history = history.read(app);
        assert!(
            history.pending_history_selections.is_empty(),
            "chip click must not select its commit"
        );
        assert_eq!(
            history
                .active_repo()
                .unwrap()
                .history_state
                .selected_commit
                .as_ref(),
            Some(&CommitId("tip".into()))
        );
    });
    wait_for_open(cx, &store, "qa-b");
}

#[gpui::test]
fn clean_worktree_chip_compacts_and_keeps_its_tooltip_and_click(cx: &mut gpui::TestAppContext) {
    assert_clean_worktree_chip_compacts_on_resize(cx, 100);
}

#[gpui::test]
fn clean_worktree_chip_compaction_scales_at_150_percent(cx: &mut gpui::TestAppContext) {
    assert_clean_worktree_chip_compacts_on_resize(cx, 150);
}

#[test]
fn clean_worktree_chip_label_does_not_repeat_an_identical_branch_and_folder() {
    let chips = history_worktree_chips_by_head(
        &[worktree("/wt/qa-b", Some("base"), Some("qa-b"))],
        Path::new("/wt/main"),
    );
    assert_eq!(chips[&CommitId("base".into())][0].label.as_ref(), "qa-b");
}
