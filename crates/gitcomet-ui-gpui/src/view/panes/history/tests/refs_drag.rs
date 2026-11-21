use super::*;
use crate::view::rows::branch_drag::{BranchDrag, BranchDragPreview};

struct Fixture {
    root: Entity<GitCometView>,
    store: AppStore,
    mode: HistoryBranchNamesMode,
}

fn mount(
    cx: &mut gpui::TestAppContext,
    mode: HistoryBranchNamesMode,
) -> (Fixture, &mut gpui::VisualTestContext) {
    let (root, cx, mut state, store) = refresh::mount(
        cx,
        Arc::new(log_page(
            vec![
                commit("tip", &["base"], "feature"),
                commit("base", &[], "main"),
            ],
            None,
        )),
    );
    let repo = &mut state.repos[0];
    repo.head_branch = Loadable::Ready("main".into());
    repo.head_branch_rev += 1;
    repo.branches = Loadable::Ready(Arc::new(vec![
        branch("main", "base"),
        branch("a", "tip"),
        branch("b", "tip"),
    ]));
    repo.branches_rev += 1;
    repo.remote_branches = Loadable::Ready(Arc::new(vec![remote_branch("origin", "r", "tip")]));
    repo.remote_branches_rev += 1;
    repo.tags = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Tag {
        name: "v1".into(),
        target: CommitId("tip".into()),
    }]));
    repo.tags_rev += 1;
    repo.history_state.selected_commit = Some(CommitId("base".into()));
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    ensure_history_cache_for_tests(cx, &root, Arc::new(state));
    cx.simulate_resize(size(px(1600.0), px(900.0)));
    cx.update(|window, app| {
        let history = root.read(app).main_pane.read(app).history_view.clone();
        history.update(app, |history, cx| {
            history.history_branch_names = mode;
            history.history_col_branch = px(320.0);
            history.history_col_graph = px(100.0);
            history.history_show_author = false;
            history.history_show_date = false;
            history.history_show_sha = false;
            history.history_show_tags = true;
            window.activate();
            window.focus(&history.history_panel_focus_handle, cx);
            cx.notify();
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    (Fixture { root, store, mode }, cx)
}

impl Fixture {
    fn sidebar_point(&self, cx: &mut gpui::VisualTestContext, name: &str) -> Point<Pixels> {
        let ix = cx.update(|_, app| {
            let pane = self.root.read(app).sidebar_pane.clone();
            pane.update(app, |pane, _| {
                pane.branch_sidebar_presentation_cached().expect("branch sidebar presentation")
                    .rows.iter().position(|row| matches!(row,
                        crate::view::branch_sidebar::BranchSidebarRow::Branch { name: candidate, section: BranchSection::Local, .. }
                            if candidate.as_ref() == name
                    )).expect("the fixture contains the local branch")
            })
        });
        let selector: &'static str = Box::leak(format!("branch_row_1_{ix}").into_boxed_str());
        wait_until(cx, "the actual sidebar branch hitbox", |cx| {
            cx.debug_bounds(selector).is_some()
        });
        let bounds = cx.debug_bounds(selector).unwrap();
        point(bounds.left() + px(48.0), bounds.center().y)
    }

    fn point(&self, commit: &str, target: HistoryMenuRef) -> Point<Pixels> {
        let bounds =
            HistoryView::painted_ref_bounds_for_test(RepoId(1), commit, self.mode, &target)
                .expect("the actual canvas paint must expose this visible named ref");
        assert!(bounds.size.width > px(0.0) && bounds.size.height > px(0.0));
        bounds.center()
    }

    fn branch(&self, name: &str) -> Point<Pixels> {
        self.point("tip", HistoryMenuRef::Branch(BranchMenuTarget::local(name)))
    }

    fn menu(&self, cx: &mut gpui::VisualTestContext) -> Option<PopoverKind> {
        cx.update(|_, app| crate::view::test_support::popover_kind(self.root.read(app), app))
    }

    fn assert_no_mutation(&self) {
        let state = self.store.snapshot();
        let repo = &state.repos[0];
        assert_eq!(repo.head_branch, Loadable::Ready("main".into()));
        assert_eq!(
            repo.history_state.selected_commit,
            Some(CommitId("base".into()))
        );
        assert_eq!(repo.local_actions_in_flight, 0);
    }
}

#[gpui::test]
fn sidebar_branch_drag_escape_cancels_before_graph_or_sidebar_release(
    cx: &mut gpui::TestAppContext,
) {
    let (fixture, cx) = mount(cx, HistoryBranchNamesMode::SeparateColumn);
    let source = fixture.sidebar_point(cx, "a");
    let graph_target = fixture.branch("b");
    let sidebar_target = fixture.sidebar_point(cx, "main");
    for target in [graph_target, sidebar_target] {
        start(cx, source);
        cx.update(|window, app| {
            let pane = fixture.root.read(app).sidebar_pane.read(app);
            assert!(
                pane.contains_focus(window, app),
                "the real sidebar press must hold focus"
            );
        });
        cx.simulate_keystrokes("escape");
        assert!(!cx.update(|_, app| app.has_active_drag()));
        release(cx, target);
        assert_eq!(fixture.menu(cx), None);
        fixture.assert_no_mutation();
    }
}

#[gpui::test]
fn sidebar_to_graph_drag_opens_the_exact_pair_without_git_mutation(cx: &mut gpui::TestAppContext) {
    let (fixture, cx) = mount(cx, HistoryBranchNamesMode::Inline);
    let source = fixture.sidebar_point(cx, "a");
    start(cx, source);
    release(cx, fixture.branch("b"));
    assert_eq!(
        fixture.menu(cx),
        Some(PopoverKind::BranchDropMenu {
            repo_id: RepoId(1),
            source: BranchMenuTarget::local("a"),
            target: BranchMenuTarget::local("b"),
        })
    );
    fixture.assert_no_mutation();
}

#[gpui::test]
fn graph_to_sidebar_drag_rejects_stale_source_target_and_active_repo(
    cx: &mut gpui::TestAppContext,
) {
    let (fixture, cx) = mount(cx, HistoryBranchNamesMode::SeparateColumn);
    let source = fixture.branch("a");
    let target = fixture.sidebar_point(cx, "main");
    let drag = BranchDrag {
        repo_id: RepoId(1),
        lifetime: fixture.store.snapshot().repos[0].lifetime(),
        target: BranchMenuTarget::local("a"),
    };
    let target_ref = BranchMenuTarget::local("main");
    let original = fixture.store.snapshot();
    let lifetime = original.repos[0].lifetime();
    assert!(drag.can_drop_live(&original, RepoId(1), lifetime, &target_ref));
    start(cx, source);
    release(cx, target);
    assert_eq!(
        fixture.menu(cx),
        Some(PopoverKind::BranchDropMenu {
            repo_id: RepoId(1),
            source: drag.target.clone(),
            target: target_ref.clone(),
        })
    );
    fixture.assert_no_mutation();
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();

    for stale in ["source", "target", "active_repo"] {
        fixture.store.replace_snapshot_for_test(original.clone());
        start(cx, source);
        let mut state = original.as_ref().clone();
        match stale {
            "source" => {
                state.repos[0].branches =
                    Loadable::Ready(Arc::new(vec![branch("main", "base"), branch("b", "tip")]))
            }
            "target" => {
                state.repos[0].branches =
                    Loadable::Ready(Arc::new(vec![branch("a", "tip"), branch("b", "tip")]))
            }
            _ => state.active_repo = None,
        }
        // Leave the painted sidebar frame intact to exercise the async gap.
        fixture.store.replace_snapshot_for_test(Arc::new(state));
        // This is also the can_drop predicate that gates the sidebar's
        // on_drag_over style in pinned GPUI, so invalid rows cannot highlight.
        assert!(
            !drag.can_drop_live(&fixture.store.snapshot(), RepoId(1), lifetime, &target_ref),
            "{stale}"
        );
        release(cx, target);
        assert_eq!(fixture.menu(cx), None, "{stale}");
        fixture.assert_no_mutation();
    }
}

fn start(cx: &mut gpui::VisualTestContext, source: Point<Pixels>) {
    cx.simulate_mouse_move(source, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(source, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        source + point(px(1.0), px(0.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    assert!(
        !cx.update(|_, app| app.has_active_drag()),
        "subthreshold movement remains a click"
    );
    cx.simulate_mouse_move(
        source + point(px(4.0), px(0.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    assert!(cx.update(|_, app| {
        app.active_drag()
            .is_some_and(|drag| drag.value.is::<BranchDrag>())
    }));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(cx.debug_bounds("sidebar_branch_drag_preview").is_some());
}

fn release(cx: &mut gpui::VisualTestContext, target: Point<Pixels>) {
    cx.simulate_mouse_move(
        target,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(target, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(!cx.update(|_, app| app.has_active_drag()));
}

fn exact_refs(cx: &mut gpui::TestAppContext, mode: HistoryBranchNamesMode) {
    let (fixture, cx) = mount(cx, mode);
    let target = fixture.branch("b");
    let local = BranchMenuTarget::local("a");
    let remote = BranchMenuTarget::remote("origin", "r");
    for source in [local, remote] {
        let position = fixture.point("tip", HistoryMenuRef::Branch(source.clone()));
        start(cx, position);
        release(cx, target);
        assert_eq!(
            fixture.menu(cx),
            Some(PopoverKind::BranchDropMenu {
                repo_id: RepoId(1),
                source,
                target: BranchMenuTarget::local("b")
            })
        );
        fixture.assert_no_mutation();
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
    }
    start(cx, fixture.branch("a"));
    release(
        cx,
        fixture.point(
            "base",
            HistoryMenuRef::Branch(BranchMenuTarget::local("main")),
        ),
    );
    assert_eq!(
        fixture.menu(cx),
        Some(PopoverKind::BranchDropMenu {
            repo_id: RepoId(1),
            source: BranchMenuTarget::local("a"),
            target: BranchMenuTarget::local("main"),
        })
    );
    fixture.assert_no_mutation();
}

#[gpui::test]
fn separate_graph_drag_keeps_exact_local_and_remote_refs_on_the_same_commit(
    cx: &mut gpui::TestAppContext,
) {
    exact_refs(cx, HistoryBranchNamesMode::SeparateColumn);
}

#[gpui::test]
fn inline_graph_drag_keeps_exact_local_and_remote_refs_on_the_same_commit(
    cx: &mut gpui::TestAppContext,
) {
    exact_refs(cx, HistoryBranchNamesMode::Inline);
}

#[gpui::test]
fn graph_drag_rejects_self_stale_refs_cross_repo_and_escape(cx: &mut gpui::TestAppContext) {
    let (fixture, cx) = mount(cx, HistoryBranchNamesMode::SeparateColumn);
    let source = fixture.branch("a");
    let target = fixture.branch("b");
    start(cx, source);
    release(cx, source);
    assert_eq!(fixture.menu(cx), None);
    fixture.assert_no_mutation();

    start(cx, source);
    cx.simulate_keystrokes("escape");
    assert!(!cx.update(|_, app| app.has_active_drag()));
    release(cx, source);
    assert_eq!(fixture.menu(cx), None);
    fixture.assert_no_mutation();

    // The target belongs to another repository even though its ref name is valid.
    cx.update(|window, app| {
        let drag = BranchDrag {
            repo_id: RepoId(2),
            lifetime: fixture.store.snapshot().repos[0].lifetime(),
            target: BranchMenuTarget::local("a"),
        };
        let preview = app.new(|_| {
            BranchDragPreview::new(&drag, AppTheme::gitcomet_dark(), point(px(0.0), px(0.0)))
        });
        app.start_drag(gpui::AnyDrag::new(drag, preview));
        window.refresh();
    });
    release(cx, target);
    assert_eq!(fixture.menu(cx), None);
    fixture.assert_no_mutation();

    // Keep the painted frame stale deliberately: live store validation must
    // reject a ref removed after the mouse was pressed.
    let original = fixture.store.snapshot();
    start(cx, source);
    let mut state = original.as_ref().clone();
    state.repos[0].branches =
        Loadable::Ready(Arc::new(vec![branch("main", "base"), branch("a", "tip")]));
    state.repos[0].branches_rev += 1;
    fixture.store.replace_snapshot_for_test(Arc::new(state));
    release(cx, target);
    assert_eq!(fixture.menu(cx), None);
    fixture.assert_no_mutation();

    fixture.store.replace_snapshot_for_test(original.clone());
    start(cx, source);
    let mut state = original.as_ref().clone();
    state.repos[0].branches =
        Loadable::Ready(Arc::new(vec![branch("main", "base"), branch("b", "tip")]));
    state.repos[0].branches_rev += 1;
    fixture.store.replace_snapshot_for_test(Arc::new(state));
    release(cx, target);
    assert_eq!(fixture.menu(cx), None);
    fixture.assert_no_mutation();

    fixture.store.replace_snapshot_for_test(original.clone());
    start(cx, source);
    let mut state = original.as_ref().clone();
    state.active_repo = None;
    fixture.store.replace_snapshot_for_test(Arc::new(state));
    release(cx, target);
    assert_eq!(fixture.menu(cx), None);
    fixture.assert_no_mutation();
}

#[gpui::test]
fn graph_drag_ignores_tags_commit_nodes_modifiers_and_overlays(cx: &mut gpui::TestAppContext) {
    let (fixture, cx) = mount(cx, HistoryBranchNamesMode::Inline);
    let source = fixture.branch("a");
    let tag = fixture.point("tip", HistoryMenuRef::Tag("v1".into()));
    let row = cx.debug_bounds("history_row_0").unwrap();
    let graph = cx.debug_bounds("history_graph_header_cell").unwrap();
    for position in [tag, point(graph.center().x, row.center().y)] {
        cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(
            position,
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_move(
            position + point(px(4.0), px(0.0)),
            Some(gpui::MouseButton::Left),
            gpui::Modifiers::default(),
        );
        assert!(!cx.update(|_, app| app.has_active_drag()));
        // Release outside the row so this probe does not select a commit.
        release(cx, point(position.x, row.top() - px(2.0)));
    }
    let shift = gpui::Modifiers {
        shift: true,
        ..Default::default()
    };
    cx.simulate_mouse_down(source, gpui::MouseButton::Left, shift);
    cx.simulate_mouse_move(
        source + point(px(4.0), px(0.0)),
        Some(gpui::MouseButton::Left),
        shift,
    );
    assert!(!cx.update(|_, app| app.has_active_drag()));
    release(cx, point(source.x, row.top() - px(2.0)));
    fixture.assert_no_mutation();

    // A context-menu header covering a chip wins the frame's hitbox test.
    let kind = PopoverKind::CommitMenu {
        repo_id: RepoId(1),
        commit_id: CommitId("tip".into()),
        refs: CommitMenuRefs::Chooser,
    };
    cx.update(|window, app| {
        fixture.root.update(app, |root, cx| {
            root.open_popover_at(kind.clone(), source - point(px(2.0), px(2.0)), window, cx)
        })
    });
    cx.run_until_parked();
    cx.simulate_mouse_down(source, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_move(
        source + point(px(4.0), px(0.0)),
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    assert!(!cx.update(|_, app| app.has_active_drag()));
    cx.simulate_mouse_up(source, gpui::MouseButton::Left, gpui::Modifiers::default());
    fixture.assert_no_mutation();
}
