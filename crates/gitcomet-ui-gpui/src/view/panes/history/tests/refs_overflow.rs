use super::*;

#[test]
fn overflow_menu_highlights_only_the_exact_repo_and_commit() {
    use crate::view::history_refs_hover::HISTORY_REFS_PICKER_MENU_INVOKER_PREFIX;
    use crate::view::rows::history_row_menu_is_open;

    let invoker = format!("{HISTORY_REFS_PICKER_MENU_INVOKER_PREFIX}12_abcd");
    assert!(history_row_menu_is_open(&invoker, RepoId(12), "abcd"));
    assert!(!history_row_menu_is_open(&invoker, RepoId(1), "abcd"));
    assert!(!history_row_menu_is_open(&invoker, RepoId(12), "abc"));
}

#[gpui::test]
fn separate_column_overflow_click_opens_chooser_without_selecting_or_checkout(
    cx: &mut gpui::TestAppContext,
) {
    assert_overflow_click(cx, HistoryBranchNamesMode::SeparateColumn, false);
}

#[gpui::test]
fn inline_overflow_click_opens_chooser_without_selecting_or_checkout(
    cx: &mut gpui::TestAppContext,
) {
    assert_overflow_click(cx, HistoryBranchNamesMode::Inline, false);
}

#[gpui::test]
fn dismissing_overflow_after_structural_change_restores_history_keyboard_focus(
    cx: &mut gpui::TestAppContext,
) {
    assert_overflow_click(cx, HistoryBranchNamesMode::SeparateColumn, true);
}

#[gpui::test]
fn dismissing_inline_overflow_after_structural_change_restores_history_keyboard_focus(
    cx: &mut gpui::TestAppContext,
) {
    assert_overflow_click(cx, HistoryBranchNamesMode::Inline, true);
}

fn assert_painted_overflow_emphasis(
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
    mode: HistoryBranchNamesMode,
    expected: bool,
) {
    // The hook records actual canvas paint, so force a fresh frame rather than
    // inspecting an emphasis value left over from a cached view.
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    assert_eq!(
        HistoryView::painted_overflow_emphasis_for_test(repo_id, "tip", mode),
        Some(expected)
    );
}

fn assert_overflow_click(
    cx: &mut gpui::TestAppContext,
    mode: HistoryBranchNamesMode,
    verify_focus_restore: bool,
) {
    let page = Arc::new(log_page(
        vec![
            commit("tip", &["base"], "feature tip"),
            commit("base", &[], "main"),
        ],
        None,
    ));
    let (view, cx, mut state, store) = refresh::mount(cx, page);
    let repo_id = RepoId(1);
    state.repos[0].head_branch = Loadable::Ready("main".into());
    state.repos[0].head_branch_rev += 1;
    let mut branches = vec![branch("main", "base")];
    branches.extend((0..20).map(|i| branch(&format!("feature-with-a-long-name-{i}"), "tip")));
    state.repos[0].branches = Loadable::Ready(Arc::new(branches));
    state.repos[0].branches_rev += 1;
    state.repos[0].history_state.selected_commit = Some(CommitId("base".into()));
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    wait_until(cx, "overflow row", |cx| {
        cx.debug_bounds("history_row_0").is_some()
    });
    let row = cx.debug_bounds("history_row_0").unwrap();
    cx.update(|window, app| {
        let pad = window.rem_size() * 0.5;
        let history = view.read(app).main_pane.read(app).history_view.clone();
        history.update(app, |history, cx| {
            history.history_branch_names = mode;
            history.history_col_branch = if verify_focus_restore {
                px(160.0)
            } else {
                px(60.0)
            };
            history.history_show_author = false;
            history.history_show_date = false;
            history.history_show_sha = false;
            // Leave only 90px for inline refs and text. This makes the refs
            // budget too small for a named chip, so the first marker is +20.
            if mode == HistoryBranchNamesMode::Inline {
                history.history_col_graph = row.size.width - pad * 2.0 - px(90.0);
            }
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let row = cx.debug_bounds("history_row_0").unwrap();
    // Read geometry emitted by this actual row paint, after layout and the
    // content-mask intersection. Headers and canvas columns need not align.
    let badge = HistoryView::painted_overflow_bounds_for_test(repo_id, "tip", mode)
        .expect("the painted fixture must contain a visible overflow badge");
    assert!(badge.size.width > px(0.0) && badge.size.height > px(0.0));
    assert_eq!(badge.intersect(&row), badge);
    assert_painted_overflow_emphasis(cx, repo_id, mode, false);
    let overflow = badge.center();
    let outside = point(overflow.x, row.top() - px(2.0));
    // Both endpoints must belong to this badge. A press outside cannot open
    // it, and releasing a badge press outside cannot open it either.
    for (pressed, released) in [(outside, overflow), (overflow, outside)] {
        cx.simulate_mouse_move(pressed, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(pressed, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            released,
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            released,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(
                crate::view::test_support::popover_kind(view.read(app), app),
                None
            )
        });
    }
    if verify_focus_restore {
        cx.update(|window, app| {
            app.clear_key_bindings();
            crate::app::bind_app_keys_for_test(app);
            crate::app::bind_text_input_keys_for_test(app);
            window.activate();
            let history = view.read(app).main_pane.read(app).history_view.clone();
            let focus = history.read(app).history_panel_focus_handle.clone();
            window.focus(&focus, app);
            let _ = window.draw(app);
        });
    }
    let selected_before = store.snapshot().repos[0]
        .history_state
        .selected_commit
        .clone();
    cx.simulate_mouse_move(overflow, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        overflow,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        overflow,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|_, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::CommitMenu {
                repo_id,
                commit_id: CommitId("tip".into()),
                refs: CommitMenuRefs::Chooser,
            })
        );
    });
    assert_eq!(
        store.snapshot().repos[0].history_state.selected_commit,
        selected_before
    );
    assert_painted_overflow_emphasis(cx, repo_id, mode, true);
    if verify_focus_restore {
        cx.update(|window, app| {
            assert!(
                window
                    .context_stack()
                    .iter()
                    .any(|context| context.contains("ContextMenu")),
                "the badge picker must own keyboard focus"
            );
            let history_entity = view.read(app).main_pane.read(app).history_view.clone();
            let history = history_entity.read(app);
            assert!(!history.history_panel_focus_handle.is_focused(window));
        });
        // Exercise the structural dismissal used by refs/log refresh directly.
        // A changed HistoryView fingerprint defers to this root dismissal path.
        cx.update(|_, app| {
            view.update(app, |view, cx| view.dismiss_history_refs_menus(cx));
        });
        cx.run_until_parked();
        assert_painted_overflow_emphasis(cx, repo_id, mode, false);
        cx.update(|window, app| {
            assert_eq!(
                crate::view::test_support::popover_kind(view.read(app), app),
                None
            );
            let history_entity = view.read(app).main_pane.read(app).history_view.clone();
            let history = history_entity.read(app);
            assert!(history.history_panel_focus_handle.is_focused(window));
        });
        assert_eq!(selected_before, Some(CommitId("base".into())));
        cx.simulate_keystrokes("up");
        wait_for_store(cx, &view, "history up after picker dismissal", || {
            store.snapshot().repos[0].history_state.selected_commit == Some(CommitId("tip".into()))
        });
        cx.simulate_keystrokes("down");
        wait_for_store(cx, &view, "history down after picker dismissal", || {
            store.snapshot().repos[0].history_state.selected_commit == Some(CommitId("base".into()))
        });
        assert_eq!(store.snapshot().repos[0].local_actions_in_flight, 0);
        return;
    }
    // A double click must still resolve to the chooser, never a branch.
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_painted_overflow_emphasis(cx, repo_id, mode, false);
    cx.simulate_event(gpui::MouseDownEvent {
        position: overflow,
        button: gpui::MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    cx.simulate_event(gpui::MouseUpEvent {
        position: overflow,
        button: gpui::MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::CommitMenu {
                repo_id,
                commit_id: CommitId("tip".into()),
                refs: CommitMenuRefs::Chooser,
            })
        );
    });
    assert_eq!(
        store.snapshot().repos[0].history_state.selected_commit,
        selected_before
    );
    assert_painted_overflow_emphasis(cx, repo_id, mode, true);
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_painted_overflow_emphasis(cx, repo_id, mode, false);
    let shift = gpui::Modifiers {
        shift: true,
        ..Default::default()
    };
    cx.simulate_mouse_down(overflow, gpui::MouseButton::Left, shift);
    cx.simulate_mouse_up(overflow, gpui::MouseButton::Left, shift);
    cx.run_until_parked();
    cx.update(|_, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            None
        )
    });
    assert_eq!(store.snapshot().repos[0].local_actions_in_flight, 0);
    assert_eq!(
        store.snapshot().repos[0].head_branch,
        Loadable::Ready("main".into())
    );
}

#[gpui::test]
fn remote_only_same_short_name_overflow_preserves_both_exact_remote_targets(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|app| {
        crate::appearance::pin_compact_for_test(app);
        app.set_global(crate::localization::Language::English);
    });
    let (view, cx, mut state, store) = refresh::mount(
        cx,
        Arc::new(log_page(
            vec![
                commit("tip", &["base"], "remote topic"),
                commit("base", &[], "main"),
            ],
            None,
        )),
    );
    let repo_id = RepoId(1);
    state.repos[0].head_branch = Loadable::Ready("main".into());
    state.repos[0].head_branch_rev += 1;
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![branch("main", "base")]));
    state.repos[0].branches_rev += 1;
    state.repos[0].remote_branches = Loadable::Ready(Arc::new(vec![
        remote_branch("origin", "topic", "tip"),
        remote_branch("peer", "topic", "tip"),
    ]));
    state.repos[0].remote_branches_rev += 1;
    state.repos[0].history_state.selected_commit = Some(CommitId("base".into()));
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    cx.update(|_, app| {
        let history = view.read(app).main_pane.read(app).history_view.clone();
        history.update(app, |history, cx| {
            history.history_branch_names = HistoryBranchNamesMode::SeparateColumn;
            // A short "topic" could fit, but neither full remote-qualified
            // label fits without truncation, so both must use the chooser.
            history.history_col_branch = px(80.0);
            history.set_history_column_preferences(true, false, false, false, cx);
            cx.notify();
        });
    });
    wait_until(cx, "remote-only qualified labels overflow", |cx| {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
        HistoryView::painted_overflow_bounds_for_test(
            repo_id,
            "tip",
            HistoryBranchNamesMode::SeparateColumn,
        )
        .is_some()
    });
    let badge = HistoryView::painted_overflow_bounds_for_test(
        repo_id,
        "tip",
        HistoryBranchNamesMode::SeparateColumn,
    )
    .unwrap();
    cx.simulate_mouse_move(badge.center(), None, gpui::Modifiers::default());
    cx.simulate_click(badge.center(), gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    cx.update(|_, app| {
        assert_eq!(
            crate::view::test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::CommitMenu {
                repo_id,
                commit_id: CommitId("tip".into()),
                refs: CommitMenuRefs::Chooser
            }),
        )
    });
    let group = |remote: &str| -> &'static str {
        format!(
            "history_ref_group_{:?}",
            HistoryMenuRef::Branch(BranchMenuTarget::remote(remote, "topic"))
        )
        .leak()
    };
    for remote in ["origin", "peer"] {
        assert!(
            cx.debug_bounds(group(remote)).is_some(),
            "{remote}/topic must be individually reachable"
        );
    }
    for (remote, other) in [("origin", "peer"), ("peer", "origin")] {
        let bounds = cx.debug_bounds(group(remote)).unwrap();
        cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
        crate::view::test_support::wait_for_native_tooltip(cx);
        let tooltip =
            crate::view::test_support::tooltip_text(cx, &view).expect("full remote group label");
        assert!(tooltip.contains(&format!("{remote}/topic")), "{tooltip}");
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        crate::view::test_support::redraw(cx);
        let action: &'static str =
            format!("context_menu_mark_{remote}_topic_for_comparison").leak();
        let wrong_action: &'static str =
            format!("context_menu_mark_{other}_topic_for_comparison").leak();
        assert!(
            cx.debug_bounds(action).is_some(),
            "expanded exact remote target {remote}/topic"
        );
        assert!(
            cx.debug_bounds(wrong_action).is_none(),
            "the other remote must remain collapsed"
        );
        // Collapse the group before expanding its neighbour. No branch action
        // is invoked; the differing typed ref drives the expanded contents.
        let bounds = cx.debug_bounds(group(remote)).unwrap();
        cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(510));
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        assert!(cx.debug_bounds(action).is_none());
    }
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    let snapshot = store.snapshot();
    assert_eq!(
        snapshot.repos[0].history_state.selected_commit,
        Some(CommitId("base".into()))
    );
    assert_eq!(
        snapshot.repos[0].head_branch,
        Loadable::Ready("main".into())
    );
    assert_eq!(snapshot.repos[0].local_actions_in_flight, 0);
    assert!(snapshot.repos[0].navigation.comparison_mark.is_none());
}

#[gpui::test]
fn overflow_branch_double_click_checks_out_only_the_exact_same_tip_branch(
    cx: &mut gpui::TestAppContext,
) {
    use crate::view::panels::ContextMenuAction;

    cx.update(|app| {
        crate::appearance::pin_compact_for_test(app);
        app.set_global(crate::localization::Language::English);
    });
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
    let branches = ["same-tip-branch-a", "same-tip-branch-b"];
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
    for name in branches {
        git(&["branch", name]);
    }
    let sha = git(&["rev-parse", "HEAD"]);
    let refs_before = git(&["show-ref", "--heads"]);
    let repository = gitcomet_git_gix::GixBackend.open(dir.path()).unwrap();
    let workdir = repository.spec().workdir.clone();
    let (view, cx, mut state, store) = refresh::mount(
        cx,
        Arc::new(log_page(vec![commit(&sha, &[], "fixture")], None)),
    );
    let repo = &mut state.repos[0];
    let repo_id = repo.id;
    repo.spec = repository.spec().clone();
    repo.head_branch = Loadable::Ready("main".into());
    repo.head_branch_rev += 1;
    repo.branches = Loadable::Ready(Arc::new(
        std::iter::once("main")
            .chain(branches)
            .map(|name| branch(name, &sha))
            .collect(),
    ));
    repo.branches_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    store.insert_repo_for_test(repo_id, repository);
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    cx.simulate_resize(size(px(1500.0), px(900.0)));
    let mode = HistoryBranchNamesMode::SeparateColumn;
    cx.update(|window, app| {
        let history = view.read(app).main_pane.read(app).history_view.clone();
        history.update(app, |history, cx| {
            history.history_branch_names = mode;
            history.history_col_branch = px(60.0);
            window.activate();
            window.focus(&history.history_panel_focus_handle, cx);
            cx.notify();
        });
    });
    crate::view::test_support::redraw(cx);
    let badge = HistoryView::painted_overflow_bounds_for_test(repo_id, &sha, mode)
        .expect("the same-tip branches must overflow into the chooser");
    let groups = branches.map(|name| -> &'static str {
        let target = HistoryMenuRef::Branch(BranchMenuTarget::local(name));
        assert!(HistoryView::painted_ref_bounds_for_test(repo_id, &sha, mode, &target).is_none());
        format!("history_ref_group_{target:?}").leak()
    });
    let chooser = PopoverKind::CommitMenu {
        repo_id,
        commit_id: CommitId(sha.as_str().into()),
        refs: CommitMenuRefs::Chooser,
    };
    let open_chooser = |cx: &mut gpui::VisualTestContext| {
        cx.simulate_mouse_move(badge.center(), None, gpui::Modifiers::default());
        cx.simulate_click(badge.center(), gpui::Modifiers::default());
        crate::view::test_support::redraw(cx);
        cx.update(|_, app| {
            assert_eq!(
                crate::view::test_support::popover_kind(view.read(app), app),
                Some(chooser.clone())
            );
        });
    };
    let group_position = |cx: &mut gpui::VisualTestContext, group: &'static str| {
        cx.debug_bounds(group).expect(group).center()
    };
    let click_at = |cx: &mut gpui::VisualTestContext, position, click_count| {
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
        crate::view::test_support::redraw(cx);
    };
    let click_group = |cx: &mut gpui::VisualTestContext, group, click_count| {
        let position = group_position(cx, group);
        click_at(cx, position, click_count);
    };
    let assert_expanded = |cx: &mut gpui::VisualTestContext, expected: &str| {
        let checkouts = cx.update(|_, app| {
            let host = view.read(app).popover_host.clone();
            host.update(app, |host, cx| {
                host.context_menu_actions_for_test(&chooser, cx)
                    .into_iter()
                    .filter_map(|action| match action {
                        ContextMenuAction::CheckoutBranch { name, .. } => Some(name),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
        });
        assert_eq!(checkouts, [expected]);
    };
    let settle_single_click = |cx: &mut gpui::VisualTestContext| {
        cx.executor().advance_clock(Duration::from_millis(510));
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
    };
    let assert_still_main = || {
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(git(&["symbolic-ref", "HEAD"]), "refs/heads/main");
        assert_eq!(git(&["show-ref", "--heads"]), refs_before);
    };
    open_chooser(cx);
    click_group(cx, groups[0], 1);
    assert_still_main();
    assert_expanded(cx, branches[0]);
    // The same SHA does not complete a gesture that started on another ref.
    click_group(cx, groups[1], 2);
    assert_still_main();

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    open_chooser(cx);
    click_group(cx, groups[0], 1);
    assert_expanded(cx, branches[0]);
    click_group(cx, groups[1], 1);
    assert_expanded(cx, branches[0]);
    settle_single_click(cx);
    assert_expanded(cx, branches[1]);
    assert_still_main();
    click_group(cx, groups[0], 1);
    settle_single_click(cx);
    assert_expanded(cx, branches[0]);

    // Both clicks stay where the pointer landed before the first click. A
    // repaint in between must not move B by collapsing the section above it.
    let position = group_position(cx, groups[1]);
    click_at(cx, position, 1);
    assert_expanded(cx, branches[0]);
    assert_eq!(group_position(cx, groups[1]), position);
    assert_still_main();
    cx.simulate_event(gpui::MouseDownEvent {
        position,
        button: gpui::MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    settle_single_click(cx);
    assert_expanded(cx, branches[0]);
    assert_eq!(group_position(cx, groups[1]), position);
    cx.simulate_event(gpui::MouseUpEvent {
        position,
        button: gpui::MouseButton::Left,
        click_count: 2,
        ..Default::default()
    });
    wait_until(cx, "the chooser double click checks out branch b", |_| {
        git(&["symbolic-ref", "HEAD"]) == format!("refs/heads/{}", branches[1])
    });
    assert_eq!(git(&["rev-parse", "HEAD"]), sha);
    assert_eq!(git(&["show-ref", "--heads"]), refs_before);
    let snapshot = store.snapshot();
    assert_eq!(snapshot.repos.len(), 1);
    assert_eq!(snapshot.repos[0].spec.workdir, workdir);
}
