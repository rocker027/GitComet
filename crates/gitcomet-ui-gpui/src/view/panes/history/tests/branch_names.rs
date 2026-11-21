use super::*;
use crate::view::panels::HistoryMenuRef;
use palette::IntoColor as _;

const REPEATED_AUTHOR_PX: f32 = 16.0;
const EXPANDED_AUTHOR_PX: f32 = 22.0;

fn redraw(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

fn graph_node_at(
    quads: &[gpui::Quad],
    graph: Bounds<Pixels>,
    row_y: Pixels,
    scale: f32,
) -> Option<&gpui::Quad> {
    quads.iter().find(|quad| {
        let center = quad.bounds.center();
        center.x.0 >= f32::from(graph.left()) * scale
            && center.x.0 < f32::from(graph.right()) * scale
            && (center.y.0 - f32::from(row_y) * scale).abs() < 0.6
            && !quad.background.is_transparent()
            && quad.border_widths.top.0 > 0.0
            && quad.bounds.size.width == quad.bounds.size.height
            && (quad.corner_radii.top_left.0 * 2.0 - quad.bounds.size.width.0).abs() < 0.1
    })
}

/// Compare solid scene colours in RGB because a channel's interpolation can
/// round at the endpoint before GPUI converts the result to HSLA.
fn solid_color_error(a: gpui::Background, b: gpui::Background) -> f32 {
    let a: gpui::Rgba = a.as_solid().expect("solid color").into_color();
    let b: gpui::Rgba = b.as_solid().expect("solid color").into_color();
    [
        (a.red - b.red).abs(),
        (a.green - b.green).abs(),
        (a.blue - b.blue).abs(),
        (a.alpha - b.alpha).abs(),
    ]
    .into_iter()
    .fold(0.0, f32::max)
}

fn tip_ref_marker(mode: HistoryBranchNamesMode) -> Bounds<Pixels> {
    HistoryView::painted_ref_bounds_for_test(
        RepoId(1),
        "tip",
        mode,
        &HistoryMenuRef::Branch(BranchMenuTarget::local("main")),
    )
    .or_else(|| HistoryView::painted_overflow_bounds_for_test(RepoId(1), "tip", mode))
    .unwrap_or_else(|| panic!("{mode:?}: the tip must paint a branch or an overflow marker"))
}

/// Inspect the production canvas output: a leader must use the node's actual
/// colour and facing edge, and initials must move out of the author column.
fn assert_ref_node_paint(
    cx: &mut gpui::VisualTestContext,
    mode: HistoryBranchNamesMode,
    graph_visible: bool,
) {
    let row = cx.debug_bounds("history_row_0").unwrap();
    let author = cx
        .debug_bounds("history_author_header_cell")
        .expect("visible author column");
    let graph = cx.debug_bounds("history_graph_header_cell");
    assert_eq!(graph.is_some(), graph_visible);
    let message = cx.debug_bounds("history_message_header_cell").unwrap();
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
        let scale = window.scale_factor();
        let quads = window.painted_quads();
        let at_row = |q: &&gpui::Quad| {
            (q.bounds.center().y.0 - f32::from(row.center().y) * scale).abs() < 0.6
        };
        if let Some(graph) = graph {
            let node = graph_node_at(&quads, graph, row.center().y, scale)
                .expect("the graph column contains the tip's author node");
            assert_eq!(node.bounds.size.width.0 / scale, EXPANDED_AUTHOR_PX);
            let marker = tip_ref_marker(mode);
            let lane_marker = quads
                .iter()
                .filter(at_row)
                .find(|quad| {
                    (quad.bounds.left().0 - f32::from(message.left()) * scale).abs() < 0.6
                        && (quad.bounds.size.width.0
                            - crate::view::HISTORY_MESSAGE_BORDER_W_PX * scale)
                            .abs() < 0.6
                        && (quad.bounds.size.height.0 - f32::from(row.size.height) * scale)
                            .abs() < 0.6
                })
                .expect("the message edge shows this row's lane color");
            let left = f32::from(graph.left().min(marker.left())) * scale;
            let right = f32::from(graph.right().max(marker.right())) * scale;
            let dots: Vec<_> = quads
                .iter()
                .filter(at_row)
                .filter(|quad| {
                    let bounds = quad.bounds;
                    bounds.left().0 >= left
                        && bounds.right().0 <= right
                        && (bounds.right() <= node.bounds.left()
                            || bounds.left() >= node.bounds.right())
                        && bounds.size.width == bounds.size.height
                        && bounds.size.width.0 <= 1.5 * scale + 1.0
                        && quad.corner_radii.top_left.0 > 0.0
                        && quad.border_widths.top.0 == 0.0
                })
                .collect();
            assert!(
                !dots.is_empty(),
                "{mode:?}: no leader between ref marker {marker:?} and node {:?}",
                node.bounds
            );
            assert!(
                dots.iter().all(|dot| dot.background == lane_marker.background),
                "{mode:?}: the leader and message edge show the same lane color"
            );
            let ring_error = solid_color_error(node.border_color, lane_marker.background);
            assert!(
                ring_error < 1.0 / 255.0,
                "{mode:?}: tip ring and leader colors diverged ({ring_error})"
            );
            println!(
                "{mode:?}: graph={:?}, ref={marker:?}, node={:?}, dots={}, ring_exact={}, max_ring_channel_delta={ring_error}",
                graph.size, node.bounds, dots.len(), node.border_color == lane_marker.background,
            );
            match mode {
                HistoryBranchNamesMode::SeparateColumn => {
                    let nearest = dots.iter().map(|dot| dot.bounds.right()).max().unwrap();
                    assert!(
                        dots.iter()
                            .all(|dot| dot.bounds.right() <= node.bounds.left())
                    );
                    assert!((nearest.0 - node.bounds.left().0).abs() < 0.6);
                }
                HistoryBranchNamesMode::Inline => {
                    let nearest = dots.iter().map(|dot| dot.bounds.left()).min().unwrap();
                    assert!(
                        dots.iter()
                            .all(|dot| dot.bounds.left() >= node.bounds.right())
                    );
                    assert!((nearest.0 - node.bounds.right().0).abs() < 0.6);
                }
            }
            assert!(
                dots.iter().all(|dot| dot.order < node.order),
                "lane node must cover the leader"
            );
        }
        {
            let avatars = quads
                .iter()
                .filter(at_row)
                .filter(|q| {
                    q.bounds.left().0 >= f32::from(author.left()) * scale
                        && q.bounds.right().0 <= f32::from(author.right()) * scale
                        && (q.bounds.size.width.0 - 16.0 * scale).abs() < 0.1
                        && (q.bounds.size.height.0 - 16.0 * scale).abs() < 0.1
                })
                .count();
            assert_eq!(avatars, usize::from(!graph_visible));
        }
    });
}

/// Tests and benchmarks settle frames under the deterministic runtime, whose
/// clock only moves on demand, so a selection must not keep a frame pending
/// there. Pane animations follow the same runtime policy.
#[gpui::test]
fn history_selection_settles_frames_in_the_deterministic_runtime(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    let page = Arc::new(log_page(
        (0..5)
            .map(|ix| commit(&format!("c{ix}"), &[&format!("c{}", ix + 1)], "Same author"))
            .collect(),
        None,
    ));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-author-deterministic"),
        },
    );
    repo.log = Loadable::Ready(page.clone());
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    let mut state = AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    ensure_history_cache_for_tests(cx, &view, Arc::new(state.clone()));
    wait_until(cx, "history rows", |cx| {
        cx.debug_bounds("history_row_4").is_some()
    });
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let _observer = cx.update(|_, app| {
        let notified = notified.clone();
        app.observe(&history, move |_, _| notified.set(notified.get() + 1))
    });
    let frame_notifies = |cx: &mut gpui::VisualTestContext| {
        notified.set(0);
        // One update per frame: gpui merges a view's notifies until a flush.
        for _ in 0..20 {
            cx.update(|window, app| {
                window.simulate_next_frame(app);
                let _ = window.draw(app);
            });
        }
        notified.get()
    };
    assert_eq!(frame_notifies(cx), 0);
    for selected in ["c2", "c3"] {
        state.repos[0].history_state.selected_commit = Some(CommitId(selected.into()));
        set_history_view_state_for_tests(cx, &view, Arc::new(state.clone()));
        redraw(cx);
        assert_eq!(
            frame_notifies(cx),
            0,
            "selecting {selected} kept requesting history frames"
        );
    }
}

#[gpui::test]
fn history_repeated_author_dot_animates_with_cached_selection_frames(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::appearance::pin_compact_for_test);
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    cx.update(|_, app| {
        let history = view.read(app).main_pane.read(app).history_view.clone();
        history.update(app, |history, cx| {
            history.set_history_column_preferences(true, true, true, false, cx);
        });
    });
    let page = Arc::new(log_page(
        (0..5)
            .map(|ix| commit(&format!("c{ix}"), &[&format!("c{}", ix + 1)], "Same author"))
            .collect(),
        None,
    ));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-author-animation"),
        },
    );
    repo.log = Loadable::Ready(page.clone());
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    let state = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    });
    ensure_history_cache_for_tests(cx, &view, state.clone());
    wait_until(cx, "author run", |cx| {
        cx.debug_bounds("history_row_4").is_some()
    });
    // Cached views may skip selector registration during animation-only frames.
    let base_row = cx.debug_bounds("history_row_2").unwrap();
    let graph = cx.debug_bounds("history_graph_header_cell").unwrap();
    let node_width = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        let mut row = base_row;
        row.origin.y += row.size.height * (row_ix as f32 - 2.0);
        cx.update(|window, _| {
            let scale = window.scale_factor();
            // Cached frames replay their quads, so the scene holds the node a
            // reused row last painted; a progressing animation must replace it.
            let quads = window.painted_quads();
            let node = graph_node_at(&quads, graph, row.center().y, scale)
                .expect("author node in its lane");
            node.bounds.size.width.0 / scale
        })
    };
    assert_eq!(node_width(cx, 2), REPEATED_AUTHOR_PX);
    // Pane animations only run under the live runtime.
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let _cache_guard = crate::view::enable_stable_cached_views_for_test();
        let mut selected = (*state).clone();
        selected.repos[0].history_state.selected_commit = Some(CommitId("c2".into()));
        set_history_view_state_for_tests(cx, &view, Arc::new(selected.clone()));
        redraw(cx);
        assert_eq!(
            node_width(cx, 2),
            REPEATED_AUTHOR_PX,
            "selection starts from the smaller author node"
        );
        let mut row_ix = 2;
        // At 90ms, even-pixel snapping can already reach the full diameter.
        for (frame, elapsed) in [45, 135].into_iter().enumerate() {
            cx.executor().advance_clock(Duration::from_millis(elapsed));
            cx.update(|window, app| {
                window.simulate_next_frame(app);
            });
            redraw(cx);
            let width = node_width(cx, row_ix);
            assert!(width > REPEATED_AUTHOR_PX && width <= EXPANDED_AUTHOR_PX);
            if frame == 0 {
                assert!(
                    width < EXPANDED_AUTHOR_PX,
                    "an intermediate frame grows the node"
                );
                println!(
                    "repeated author: start={REPEATED_AUTHOR_PX}px, intermediate={width}px, end={EXPANDED_AUTHOR_PX}px"
                );
                selected.repos[0].worktree_status =
                    Loadable::Ready(Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: PathBuf::from("README.md"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]));
                selected.repos[0].worktree_status_rev += 1;
                set_history_view_state_for_tests(cx, &view, Arc::new(selected.clone()));
                redraw(cx);
                row_ix = 3;
                assert_eq!(
                    node_width(cx, row_ix),
                    width,
                    "inserting WIP must preserve the animation"
                );
            }
        }
        assert_eq!(node_width(cx, row_ix), EXPANDED_AUTHOR_PX);
        selected.repos[0].worktree_status = Loadable::Ready(Arc::new(Vec::new()));
        selected.repos[0].worktree_status_rev += 1;
        set_history_view_state_for_tests(cx, &view, Arc::new(selected));
        redraw(cx);
        assert_eq!(
            node_width(cx, 2),
            EXPANDED_AUTHOR_PX,
            "removing WIP retains the selected node"
        );
    });
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    // The live runtime builds the cache on a thread. The clock stands still
    // meanwhile, so a node that inherited a transition would stay mid-way.
    cx.executor().allow_parking();
    let show_repo = |cx: &mut gpui::VisualTestContext, state: AppState| {
        let repo_id = state.active_repo.unwrap();
        ensure_history_cache_for_tests(cx, &view, Arc::new(state));
        wait_until(cx, "repository history cache", |cx| {
            cx.update(|_, app| {
                history
                    .read(app)
                    .history_cache
                    .as_ref()
                    .is_some_and(|cache| cache.base.request.repo_id == repo_id)
            })
        });
        redraw(cx);
    };
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let mut other_repo = state.repos[0].clone();
        other_repo.id = RepoId(2);
        other_repo.spec.workdir = PathBuf::from("/tmp/history-author-other-worktree");
        show_repo(
            cx,
            AppState {
                repos: vec![other_repo],
                active_repo: Some(RepoId(2)),
                ..(*state).clone()
            },
        );
        assert_eq!(
            node_width(cx, 2),
            REPEATED_AUTHOR_PX,
            "another repository must not inherit selection for the same commit"
        );
        let mut returning = (*state).clone();
        returning.repos[0].history_state.selected_commit = Some(CommitId("c2".into()));
        show_repo(cx, returning);
        assert_eq!(node_width(cx, 2), EXPANDED_AUTHOR_PX);
        cx.update(|_, app| app.set_reduce_motion(true));
        set_history_view_state_for_tests(cx, &view, state);
        redraw(cx);
        assert_eq!(
            node_width(cx, 2),
            REPEATED_AUTHOR_PX,
            "reduced motion settles immediately"
        );
    });
}

#[gpui::test]
fn history_branch_names_repositions_headers_and_separates_hover_regions(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(crate::appearance::pin_compact_for_test);
    let (store, events) = AppStore::new_test(Arc::new(BlockingBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.simulate_resize(size(px(1400.0), px(900.0)));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_history_branch_names(HistoryBranchNamesMode::SeparateColumn, cx);
        });
    });
    let page = Arc::new(log_page(
        vec![
            commit("tip", &["base"], "Tip message"),
            commit("base", &[], "Base message"),
        ],
        None,
    ));
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/history-inline-refs"),
        },
    );
    repo.history_state.history_scope = LogScope::AllBranches;
    repo.head_branch = Loadable::Ready("main".into());
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
    repo.branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(
        (0..12)
            .map(|ix| gitcomet_core::domain::Tag {
                name: format!("release-{ix}"),
                target: CommitId("tip".into()),
            })
            .collect(),
    ));
    repo.tags_rev = 1;
    repo.log = Loadable::Ready(page.clone());
    repo.log_rev = 1;
    repo.history_state.log = Loadable::Ready(page);
    repo.history_state.log_rev = 1;
    ensure_history_cache_for_tests(
        cx,
        &view,
        Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(RepoId(1)),
            ..AppState::test_default()
        }),
    );
    wait_until(cx, "history rows", |cx| {
        cx.debug_bounds("history_row_1").is_some()
    });
    let history = cx.update(|_, app| view.read(app).main_pane.read(app).history_view.clone());
    let graph = cx.update(|_, app| {
        history.update(app, |history, cx| {
            history.history_col_branch = px(174.0);
            cx.notify();
            history
                .history_cache
                .as_ref()
                .unwrap()
                .base
                .graph_rows
                .clone()
        })
    });
    redraw(cx);
    assert_ref_node_paint(cx, HistoryBranchNamesMode::SeparateColumn, true);
    let original_left = cx.debug_bounds("history_ref_header_cell").unwrap().left();
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_history_branch_names(HistoryBranchNamesMode::Inline, cx)
        })
    });

    for (show_graph, graph_width) in [(true, 80.0), (true, 28.0), (false, 28.0)] {
        cx.update(|_, app| {
            history.update(app, |history, cx| {
                history.history_col_graph = px(graph_width);
                history.set_history_column_preferences(show_graph, true, true, false, cx);
                cx.notify();
            })
        });
        redraw(cx);
        assert_ref_node_paint(cx, HistoryBranchNamesMode::Inline, show_graph);
        assert!(cx.debug_bounds("history_ref_header_cell").is_none());
        let message = cx.debug_bounds("history_message_header_cell").unwrap();
        let control = cx.debug_bounds("history_mode_header").unwrap();
        if show_graph {
            let graph_cell = cx.debug_bounds("history_graph_header_cell").unwrap();
            assert_eq!(graph_cell.left(), original_left);
            assert_eq!(graph_cell.right(), message.left());
            assert!(control.left() >= graph_cell.left());
            assert!(control.right() <= graph_cell.right());
        } else {
            assert!(cx.debug_bounds("history_graph_header_cell").is_none());
            assert_eq!(message.left(), original_left);
            assert!(control.left() >= message.left());
            assert!(control.right() < message.right());
        }
        crate::view::tooltip::clear_visible_tooltip_text_for_test();
        cx.simulate_mouse_move(control.center(), None, gpui::Modifiers::default());
        crate::view::test_support::wait_for_native_tooltip(cx);
        assert_eq!(
            crate::view::test_support::tooltip_text(cx, &view).as_deref(),
            Some("History mode: All branches")
        );
        cx.simulate_click(control.center(), gpui::Modifiers::default());
        redraw(cx);
        cx.update(|_, app| {
            assert_eq!(
                crate::view::test_support::popover_kind(view.read(app), app),
                Some(PopoverKind::HistoryBranchFilter { repo_id: RepoId(1) })
            );
            let host = view.read(app).popover_host.clone();
            host.update(app, |host, cx| host.close_popover(cx));
        });
        redraw(cx);
        let row = cx.debug_bounds("history_row_0").unwrap();
        cx.simulate_mouse_move(
            tip_ref_marker(HistoryBranchNamesMode::Inline).center(),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(800));
        redraw(cx);
        assert!(cx.debug_bounds("history_refs_hover_panel").is_some());
        assert!(cx.debug_bounds("commit_message_hover").is_none());
        cx.simulate_mouse_move(
            point(message.left() + message.size.width * 0.75, row.center().y),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(800));
        redraw(cx);
        assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
        assert!(cx.debug_bounds("commit_message_hover").is_some());
        cx.update(|_, app| view.update(app, |view, cx| view.dismiss_commit_message_hover(cx)));
        redraw(cx);
        // A row with no refs uses the beginning of the cell for its message.
        let base = cx.debug_bounds("history_row_1").unwrap();
        cx.simulate_mouse_move(
            point(message.left() + px(16.0), base.center().y),
            None,
            gpui::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(800));
        redraw(cx);
        assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
        assert!(cx.debug_bounds("commit_message_hover").is_some());
        cx.update(|_, app| view.update(app, |view, cx| view.dismiss_commit_message_hover(cx)));
    }

    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_history_branch_names(HistoryBranchNamesMode::SeparateColumn, cx)
        })
    });
    redraw(cx);
    assert_eq!(
        cx.debug_bounds("history_ref_header_cell")
            .unwrap()
            .size
            .width,
        px(174.0)
    );
    assert!(cx.debug_bounds("history_refs_hover_panel").is_none());
    assert!(cx.debug_bounds("commit_message_hover").is_none());
    cx.update(|_, app| {
        let history = history.read(app);
        assert_eq!(history.history_ref_column_width(), px(174.0));
        assert!(Arc::ptr_eq(
            &graph,
            &history.history_cache.as_ref().unwrap().base.graph_rows
        ));
    });
}
