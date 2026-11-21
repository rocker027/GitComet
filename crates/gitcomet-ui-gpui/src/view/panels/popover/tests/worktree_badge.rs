use super::super::worktree_badge_picker::{self, WorktreeBadgeRow};
use super::*;
use crate::localization::Language;
use crate::view::panels::tests::{app_state_with_repo, opening_repo_state};
use crate::view::test_support::{push_test_state, redraw};
use gitcomet_core::domain::{
    Branch, CommitId, Remote, RemoteBranch, Upstream, UpstreamDivergence, Worktree,
};
use gitcomet_state::model::{Loadable, RepoId, RepoState};
use std::path::PathBuf;

fn worktree(path: &str, branch: Option<&str>, head: Option<&str>) -> Worktree {
    Worktree {
        bare: false,
        path: PathBuf::from(path),
        head: head.map(|h| CommitId(std::sync::Arc::from(h))),
        branch: branch.map(str::to_string),
        detached: branch.is_none(),
    }
}

/// A repo at `/tmp/ws/main` whose worktree list also contains two siblings.
fn repo_with_worktrees(repo_id: RepoId) -> RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/ws/main"));
    repo.head_branch = Loadable::Ready("main".to_string());
    repo.worktrees = Loadable::Ready(Arc::new(vec![
        worktree(
            "/tmp/ws/main",
            Some("main"),
            Some("399f41d0000000000000000000000000000000aa"),
        ),
        worktree(
            "/tmp/ws/feature",
            Some("feat/badges"),
            Some("a12bc3d0000000000000000000000000000000bb"),
        ),
        worktree(
            "/tmp/ws/detached",
            None,
            Some("cc9911220000000000000000000000000000cc11"),
        ),
    ]));
    repo
}

#[gpui::test]
fn worktree_folder_location_is_cached_until_menu_reopens(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("checkout");
    std::fs::create_dir(&path).unwrap();
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);
    let host = cx.update(|_window, app| view.read(app).popover_host.clone());
    let kind = PopoverKind::worktree(
        repo_id,
        WorktreePopoverKind::Menu {
            path: path.clone(),
            branch: None,
        },
    );
    let open = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            host.update(app, |host, cx| {
                host.open_popover_at(
                    kind.clone(),
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            })
        });
        cx.run_until_parked();
    };
    let disabled = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            host.update(app, |host, cx| {
                host.context_menu_model(&kind, cx)
                    .unwrap()
                    .items
                    .iter()
                    .find_map(|item| match item {
                        ContextMenuItem::Entry {
                            action, disabled, ..
                        } if matches!(
                            action.as_ref(),
                            ContextMenuAction::OpenRepositoryLocation { .. }
                        ) =>
                        {
                            Some(*disabled)
                        }
                        _ => None,
                    })
                    .unwrap()
            })
        })
    };
    open(cx);
    assert!(!disabled(cx));
    std::fs::remove_dir(&path).unwrap();
    for _ in 0..3 {
        redraw(cx);
        assert!(
            !disabled(cx),
            "repainting a menu must not check the filesystem again"
        );
    }
    open(cx);
    assert!(
        disabled(cx),
        "reopening must refresh the cached availability"
    );
}

/// Opens the workspace badge picker over a state containing `repo`.
fn open_worktree_badge_picker(
    cx: &mut gpui::TestAppContext,
    repo: RepoState,
    repo_id: RepoId,
) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::worktree(repo_id, WorktreePopoverKind::BadgePicker),
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    redraw(cx);

    (view, cx)
}

#[gpui::test]
fn worktree_badge_picker_lists_every_worktree_and_marks_the_current_one(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

    let built = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let built = worktree_badge_picker::cached(host, repo_id, "", Language::English);
        (built.payloads.to_vec(), built.marked_index)
    });
    let (rows, marked_index) = built;

    // Create row, then all three worktrees — including the current one, which
    // the Open picker deliberately hides.
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0], WorktreeBadgeRow::CreateNew);
    assert_eq!(
        rows[1],
        WorktreeBadgeRow::Worktree(PathBuf::from("/tmp/ws/main"))
    );
    assert_eq!(
        marked_index,
        Some(1),
        "the check must sit on the active worktree, indexed before filtering"
    );

    redraw(cx);
    for selector in [
        "picker_prompt_item_0",
        "picker_prompt_item_1",
        "picker_prompt_item_2",
        "picker_prompt_item_3",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "expected {selector} to render"
        );
    }
}

#[gpui::test]
fn worktree_badge_picker_switches_labels_without_changing_row_payloads(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);
    cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let english = worktree_badge_picker::cached(host, repo_id, "", Language::English);
        let chinese =
            worktree_badge_picker::cached(host, repo_id, "", Language::TraditionalChinese);
        assert_eq!(english.items[0].debug_display_text(), "Create new worktree");
        assert_eq!(chinese.items[0].debug_display_text(), "新增工作樹");
        assert_eq!(chinese.items[0].debug_secondary_text(), "起始參照 main");
        assert!(english.items[2].debug_secondary_text().contains("main / "));
        assert!(
            english.items[2]
                .debug_secondary_text()
                .contains("/tmp/ws/feature")
        );
        assert_eq!(english.payloads, chinese.payloads);
        assert_eq!(english.marked_index, chinese.marked_index);
        assert_eq!(
            worktree_badge_picker::nav_targets(
                host,
                repo_id,
                "feat/badges",
                Language::TraditionalChinese
            ),
            vec![
                WorktreeBadgeRow::CreateNew,
                WorktreeBadgeRow::Worktree(PathBuf::from("/tmp/ws/feature")),
            ]
        );
    });
}

#[gpui::test]
fn worktree_badge_picker_create_row_survives_a_query_matching_no_worktree(
    cx: &mut gpui::TestAppContext,
) {
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

    let targets = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        worktree_badge_picker::nav_targets(host, repo_id, "totally-new-thing", Language::English)
    });

    // `match_items` drops rows whose match text lacks the query, so the create
    // row has to carry the query itself.
    assert_eq!(
        targets,
        vec![WorktreeBadgeRow::CreateNew],
        "create row must stay reachable for a name that does not exist yet"
    );
}

#[gpui::test]
fn worktree_badge_picker_nav_targets_follow_the_rendered_row_order(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

    let (targets, rendered) = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        let query = "feat";
        let targets = worktree_badge_picker::nav_targets(host, repo_id, query, Language::English);
        // What the panel renders, resolved exactly the way PickerPrompt does.
        let built = worktree_badge_picker::cached(host, repo_id, query, Language::English);
        let layout = crate::view::components::picker_prompt_layout(&built.items, query);
        let rendered: Vec<_> = layout
            .item_indices
            .iter()
            .map(|ix| built.payloads[*ix].clone())
            .collect();
        (targets, rendered)
    });

    assert_eq!(
        targets, rendered,
        "keyboard order must match render order or Enter opens the wrong worktree"
    );
    assert!(targets.contains(&WorktreeBadgeRow::Worktree(PathBuf::from(
        "/tmp/ws/feature"
    ))));
}

#[gpui::test]
fn worktree_badge_picker_filters_worktrees_by_name_branch_and_path(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

    let by_branch = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        worktree_badge_picker::nav_targets(host, repo_id, "badges", Language::English)
    });

    assert_eq!(
        by_branch,
        vec![
            WorktreeBadgeRow::CreateNew,
            WorktreeBadgeRow::Worktree(PathBuf::from("/tmp/ws/feature")),
        ],
        "a branch-name query should find its worktree"
    );

    // The path sits on the row's secondary line; it must still filter from there.
    let by_path = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        worktree_badge_picker::nav_targets(host, repo_id, "ws/detached", Language::English)
    });

    assert_eq!(
        by_path,
        vec![
            WorktreeBadgeRow::CreateNew,
            WorktreeBadgeRow::Worktree(PathBuf::from("/tmp/ws/detached")),
        ],
        "a path query should find its worktree from the detail line"
    );
}

#[gpui::test]
fn worktree_badge_picker_enter_reaches_the_create_row_without_arrowing(
    cx: &mut gpui::TestAppContext,
) {
    // "Select or type to create a worktree": typing then Enter must act, even
    // though nothing was arrowed to (selection starts as None).
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

    cx.simulate_input("shiny");
    simulate_key_press(cx, "enter");
    redraw(cx);

    let kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert!(
        matches!(
            kind,
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            })
        ),
        "Enter after typing should reach the create row, got {kind:?}"
    );
}

#[gpui::test]
fn worktree_badge_picker_enter_on_empty_query_stays_inert(cx: &mut gpui::TestAppContext) {
    // A stray Enter on the freshly opened picker must not create anything.
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

    simulate_key_press(cx, "enter");
    redraw(cx);

    let kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert!(
        matches!(
            kind,
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                ..
            })
        ),
        "picker should still be open and unchanged, got {kind:?}"
    );
}

#[gpui::test]
fn worktree_badge_picker_escape_closes(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(is_open, "expected the workspace picker to open");

    simulate_key_press(cx, "escape");
    redraw(cx);

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Escape to close the workspace picker");
}

#[gpui::test]
fn worktree_badge_picker_create_row_opens_the_add_dialog_prefilled(cx: &mut gpui::TestAppContext) {
    let repo_id = RepoId(1);
    let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                worktree_badge_picker::activate(
                    host,
                    repo_id,
                    WorktreeBadgeRow::CreateNew,
                    "shiny",
                    window,
                    cx,
                );
            });
        });
    });
    redraw(cx);

    let kind = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert!(
        matches!(
            kind,
            Some(PopoverKind::Repo {
                kind: RepoPopoverKind::Worktree(WorktreePopoverKind::AddPrompt),
                ..
            })
        ),
        "create row should hand off to the Add-worktree dialog, got {kind:?}"
    );

    let (path, reference, create_branch, new_branch) = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        (
            host.worktree_path_input_text_for_tests(app),
            host.worktree_ref_source_target_for_tests().to_string(),
            host.worktree_add_create_branch,
            host.worktree_new_branch_input.read(app).text().to_string(),
        )
    });
    assert_eq!(
        PathBuf::from(path),
        PathBuf::from("/tmp/ws/main"),
        "the folder starts as the current worktree's path"
    );
    // A new branch off the head the row's "based on <head>" label names; git
    // would refuse to check `main` itself out a second time.
    assert_eq!(reference, "main");
    assert!(create_branch);
    assert_eq!(new_branch, "shiny");
}

/// The action-bar badges themselves: rendering, labels, and the popovers they open.
mod badges {
    use super::*;

    fn repo_with_upstream(repo_id: RepoId) -> RepoState {
        repo_tracking_branch(repo_id, "main")
    }

    fn repo_tracking_branch(repo_id: RepoId, branch: &str) -> RepoState {
        let mut repo = repo_with_worktrees(repo_id);
        let target = CommitId(Arc::from("1111111111111111111111111111111111111111"));
        repo.head_branch = Loadable::Ready(branch.to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: branch.to_string(),
            target: target.clone(),
            upstream: Some(Upstream {
                remote: "origin".to_string(),
                branch: branch.to_string(),
            }),
            divergence: None,
        }]));
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: branch.to_string(),
            target,
        }]));
        repo
    }

    fn repo_without_upstream(repo_id: RepoId) -> RepoState {
        let mut repo = repo_with_worktrees(repo_id);
        let target = CommitId(Arc::from("1111111111111111111111111111111111111111"));
        repo.branches = Loadable::Ready(Arc::new(vec![Branch {
            name: "main".to_string(),
            target: target.clone(),
            upstream: None,
            divergence: None,
        }]));
        repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: None,
            default_branch: None,
        }]));
        repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target,
        }]));
        repo
    }

    fn draw_with_repo(
        cx: &mut gpui::TestAppContext,
        repo: RepoState,
        repo_id: RepoId,
    ) -> (gpui::Entity<GitCometView>, &mut gpui::VisualTestContext) {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            crate::app::bind_text_input_keys_for_test(app);
            view.update(app, |this, cx| {
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
            let _ = window.draw(app);
        });
        redraw(cx);

        (view, cx)
    }

    #[gpui::test]
    fn both_badges_render_when_a_repo_is_open(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (_view, cx) = draw_with_repo(cx, repo_with_worktrees(repo_id), repo_id);

        assert!(
            cx.debug_bounds("worktree_badge").is_some(),
            "expected the workspace badge on the action bar"
        );
        assert!(
            cx.debug_bounds("branch_badge").is_some(),
            "expected the branch badge on the action bar"
        );
    }

    #[gpui::test]
    fn pull_and_push_follow_the_local_branch_without_an_upstream_badge(
        cx: &mut gpui::TestAppContext,
    ) {
        let repo_id = RepoId(1);
        let (_view, cx) = draw_with_repo(cx, repo_with_upstream(repo_id), repo_id);

        let branch = cx.debug_bounds("branch_badge").expect("branch badge");
        let pull = cx.debug_bounds("pull").expect("pull button");
        let push = cx.debug_bounds("push").expect("push button");

        assert!(cx.debug_bounds("upstream_arrow").is_none());
        assert!(cx.debug_bounds("upstream_badge").is_none());
        assert!(pull.left() >= branch.right() && pull.left() - branch.right() <= px(12.0));
        assert!(push.left() >= pull.right());
    }

    #[gpui::test]
    fn branch_badge_uses_available_width_for_the_name_suffix(cx: &mut gpui::TestAppContext) {
        let widths_for = |cx: &mut gpui::TestAppContext, name: &str| {
            let repo_id = RepoId(1);
            let (_, cx) = draw_with_repo(cx, repo_tracking_branch(repo_id, name), repo_id);
            cx.simulate_resize(gpui::size(px(1800.0), px(560.0)));
            redraw(cx);
            cx.debug_bounds("branch_badge")
                .expect("branch badge")
                .size
                .width
        };
        // The prefix is already longer than the previous 28-character limit.
        let prefix = "release/mobile-worktree-update";
        let short = widths_for(cx, prefix);
        let long = widths_for(cx, &format!("{prefix}-signing-and-provisioning"));
        assert!(
            long > short + px(100.0),
            "a longer branch should show its suffix when space is available: {short:?}, {long:?}"
        );
    }

    #[gpui::test]
    fn create_branch_and_stash_actions_sit_beside_push(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (_, cx) = draw_with_repo(cx, repo_with_upstream(repo_id), repo_id);
        cx.simulate_resize(gpui::size(px(1280.0), px(560.0)));
        redraw(cx);

        let push = cx.debug_bounds("push").expect("push button");
        let branch = cx
            .debug_bounds("create_branch")
            .expect("create branch button");
        let stash = cx
            .debug_bounds("stash_toolbar_caption")
            .expect("stash caption");
        let pop = cx.debug_bounds("stash_pop").expect("pop button");
        let restore = cx
            .debug_bounds("stash_restore_menu")
            .expect("stash restore menu");
        let terminal = cx.debug_bounds("terminal").expect("terminal button");

        assert!(branch.left() >= push.right() && branch.left() - push.right() <= px(12.0));
        assert!(stash.left() >= branch.right());
        assert!(pop.left() >= stash.right());
        assert!(restore.left() >= pop.right());
        assert!(terminal.left() >= restore.right());
    }

    #[gpui::test]
    fn pull_menu_opens_the_upstream_picker_for_an_untracked_branch(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = draw_with_repo(cx, repo_without_upstream(repo_id), repo_id);

        assert!(cx.debug_bounds("upstream_badge").is_none());
        for selector in ["pull_menu", "context_menu_manage_upstream"] {
            let center = cx.debug_bounds(selector).expect(selector).center();
            cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
            cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
            cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
            cx.run_until_parked();
            redraw(cx);
        }

        let kind = cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        });
        assert_eq!(
            kind,
            Some(PopoverKind::UpstreamPicker {
                repo_id,
                branch: "main".to_string(),
            })
        );
    }

    /// Every density must fit: Comfortable and Spacious widen every control.
    const ALL_DENSITIES: [crate::appearance::UiDensity; 3] = crate::appearance::UiDensity::ALL;

    #[gpui::test]
    fn git_toolbar_shows_seven_captions_in_a_split_screen_window(cx: &mut gpui::TestAppContext) {
        cx.update(|app| {
            crate::appearance::pin_density_for_test(app, ALL_DENSITIES[1]);
        });
        let repo_id = RepoId(1);
        let (_view, cx) = draw_with_repo(cx, repo_with_upstream(repo_id), repo_id);
        let buttons = [
            ("git_undo", "git_undo_toolbar_caption"),
            ("git_redo", "git_redo_toolbar_caption"),
            ("pull_main", "pull_main_toolbar_caption"),
            ("push_main", "push_main_toolbar_caption"),
            ("create_branch", "create_branch_toolbar_caption"),
            ("stash", "stash_toolbar_caption"),
            ("stash_pop", "stash_pop_toolbar_caption"),
        ];

        cx.simulate_resize(gpui::size(px(960.0), px(560.0)));
        for language in crate::localization::Language::ALL {
            cx.update(|window, app| {
                app.set_global(language);
                let _ = window.draw(app);
            });
            redraw(cx);
            for (button, caption) in buttons {
                let button = cx.debug_bounds(button).expect(button);
                let label = cx.debug_bounds(caption).expect(caption);
                assert!(label.top() >= button.top() + px(14.0), "{caption}");
                assert!(label.bottom() <= button.bottom(), "{caption}");
                assert!(label.left() >= button.left(), "{caption}");
                assert!(label.right() <= button.right(), "{caption}");
            }
            for (main, menu) in [
                ("pull_main", "pull_menu"),
                ("push_main", "push_menu"),
                ("stash_pop", "stash_restore_menu"),
            ] {
                let main = cx.debug_bounds(main).expect(main);
                let menu = cx.debug_bounds(menu).expect(menu);
                assert_eq!(main.top(), menu.top());
                assert_eq!(main.bottom(), menu.bottom());
                assert!(menu.left() >= main.right());
            }
        }

        cx.simulate_resize(gpui::size(px(820.0), px(560.0)));
        redraw(cx);
        for (button, caption) in buttons {
            let bounds = cx.debug_bounds(button).expect(button);
            assert!(bounds.left() >= px(0.0) && bounds.right() <= px(820.0));
            assert!(cx.debug_bounds(caption).is_none(), "{caption}");
        }
    }

    #[gpui::test]
    fn tracked_branch_actions_stay_reachable_at_supported_narrow_widths_compact(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_tracked_branch_actions_reachable(cx, ALL_DENSITIES[0]);
    }

    #[gpui::test]
    fn tracked_branch_actions_stay_reachable_at_supported_narrow_widths_comfortable(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_tracked_branch_actions_reachable(cx, ALL_DENSITIES[1]);
    }

    #[gpui::test]
    fn tracked_branch_actions_stay_reachable_at_supported_narrow_widths_spacious(
        cx: &mut gpui::TestAppContext,
    ) {
        assert_tracked_branch_actions_reachable(cx, ALL_DENSITIES[2]);
    }

    fn assert_tracked_branch_actions_reachable(
        cx: &mut gpui::TestAppContext,
        density: crate::appearance::UiDensity,
    ) {
        cx.update(|app| crate::appearance::pin_density_for_test(app, density));
        let repo_id = RepoId(1);
        let mut repo = repo_tracking_branch(repo_id, "feature/current-with-a-representative-name");
        repo.upstream_divergence = Loadable::Ready(Some(UpstreamDivergence {
            ahead: 999,
            behind: 999,
        }));
        let (_view, cx) = draw_with_repo(cx, repo, repo_id);

        // Exercise compact mode and one pixel beyond each responsive
        // breakpoint (they move out at roomier densities), where a label
        // expansion is most likely to overflow.
        let [compact_max, condensed_max] =
            crate::view::panels::action_bar_breakpoints(crate::appearance::Appearance {
                density,
                ..crate::appearance::Appearance::default()
            });
        for width in [
            820.0,
            961.0,
            compact_max.floor() + 1.0,
            condensed_max.floor() + 1.0,
        ] {
            cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(560.0)));
            redraw(cx);
            let viewport = cx.update(|window, _app| window.viewport_size());
            let left_group = cx.debug_bounds("left_action_group").expect("left group");
            let right_group = cx.debug_bounds("right_action_group").expect("right group");
            assert!(
                left_group.right() <= right_group.left(),
                "action groups must not overlap at {width}px {density:?}: {left_group:?}, {right_group:?}"
            );
            for selector in [
                "global_nav",
                "worktree_badge",
                "branch_badge",
                "pull",
                "push",
                "create_branch",
                "stash",
                "stash_pop",
                "stash_restore_menu",
                "terminal",
            ] {
                let bounds = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("missing {selector} at {width}px window width"));
                assert!(
                    bounds.left() >= gpui::px(0.0) && bounds.right() <= viewport.width,
                    "{selector} must remain inside the {viewport:?} viewport at {width}px, got {bounds:?}"
                );
                let containing_group = if selector == "terminal" {
                    right_group
                } else {
                    left_group
                };
                assert!(
                    bounds.left() >= containing_group.left()
                        && bounds.right() <= containing_group.right(),
                    "{selector} must not be clipped by its action group at {width}px {density:?}: {bounds:?} outside {containing_group:?}"
                );
            }
        }
    }

    #[gpui::test]
    fn merge_controls_stay_reachable_at_the_minimum_window_width(cx: &mut gpui::TestAppContext) {
        for density in ALL_DENSITIES {
            assert_merge_controls_reachable(cx, density);
        }
    }

    fn assert_merge_controls_reachable(
        cx: &mut gpui::TestAppContext,
        density: crate::appearance::UiDensity,
    ) {
        cx.update(|app| crate::appearance::pin_density_for_test(app, density));
        let repo_id = RepoId(1);
        let mut repo = repo_with_upstream(repo_id);
        repo.merge_commit_message = Loadable::Ready(Some("Merge topic".to_string()));
        let (_view, cx) = draw_with_repo(cx, repo, repo_id);

        cx.simulate_resize(gpui::size(gpui::px(820.0), gpui::px(560.0)));
        redraw(cx);
        let left_group = cx.debug_bounds("left_action_group").expect("left group");
        let controls = cx.debug_bounds("merge_controls").expect("merge controls");
        assert!(
            controls.left() >= left_group.left() && controls.right() <= left_group.right(),
            "merge controls must not be clipped at minimum width ({density:?}): {controls:?} outside {left_group:?}"
        );
    }

    #[gpui::test]
    fn badges_sit_after_the_global_nav_arrows(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (_view, cx) = draw_with_repo(cx, repo_with_worktrees(repo_id), repo_id);

        let forward = cx
            .debug_bounds("global_nav_forward")
            .or_else(|| cx.debug_bounds("global_nav"))
            .map(|b| b.origin.x);
        let workspace = cx
            .debug_bounds("worktree_badge")
            .expect("workspace badge")
            .origin
            .x;
        let branch = cx
            .debug_bounds("branch_badge")
            .expect("branch badge")
            .origin
            .x;

        if let Some(forward) = forward {
            assert!(
                workspace > forward,
                "workspace badge should follow the nav arrows"
            );
        }
        assert!(
            branch > workspace,
            "branch badge should follow the workspace badge"
        );
    }

    #[gpui::test]
    fn clicking_the_worktree_badge_opens_the_worktree_picker(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = draw_with_repo(cx, repo_with_worktrees(repo_id), repo_id);

        let center = cx.debug_bounds("worktree_badge").expect("badge").center();
        cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.run_until_parked();
        redraw(cx);

        let kind = cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        });
        assert!(
            matches!(
                kind,
                Some(PopoverKind::Repo {
                    kind: RepoPopoverKind::Worktree(WorktreePopoverKind::BadgePicker),
                    ..
                })
            ),
            "expected the workspace picker, got {kind:?}"
        );
    }

    #[gpui::test]
    fn clicking_the_branch_badge_opens_the_checkout_picker(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let (view, cx) = draw_with_repo(cx, repo_with_worktrees(repo_id), repo_id);

        let center = cx.debug_bounds("branch_badge").expect("badge").center();
        cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.run_until_parked();
        redraw(cx);

        let kind = cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        });
        assert_eq!(
            kind,
            Some(PopoverKind::BranchPicker {
                purpose: BranchPickerPurpose::Checkout
            }),
            "expected the checkout picker"
        );
    }

    #[gpui::test]
    fn branch_badge_is_hidden_until_the_head_branch_loads(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let mut repo = repo_with_worktrees(repo_id);
        repo.head_branch = Loadable::Loading;
        let (_view, cx) = draw_with_repo(cx, repo, repo_id);

        assert!(
            cx.debug_bounds("branch_badge").is_none(),
            "no branch badge before HEAD is known"
        );
        assert!(
            cx.debug_bounds("worktree_badge").is_some(),
            "the workspace badge does not depend on HEAD"
        );
    }

    #[gpui::test]
    fn detached_head_still_renders_a_badge(cx: &mut gpui::TestAppContext) {
        let repo_id = RepoId(1);
        let mut repo = repo_with_worktrees(repo_id);
        // Detached HEAD surfaces as the literal string "HEAD".
        repo.head_branch = Loadable::Ready("HEAD".to_string());
        let (_view, cx) = draw_with_repo(cx, repo, repo_id);

        assert!(
            cx.debug_bounds("branch_badge").is_some(),
            "a detached HEAD should still offer the checkout picker"
        );
    }

    #[gpui::test]
    fn no_badges_without_an_active_repo(cx: &mut gpui::TestAppContext) {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (_view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        redraw(cx);

        assert!(cx.debug_bounds("worktree_badge").is_none());
        assert!(cx.debug_bounds("branch_badge").is_none());
    }
    /// Right-clicking a worktree row floats the very menu that worktree's sidebar
    /// row opens — asserted against that model, so the two cannot drift apart.
    #[gpui::test]
    fn right_clicking_a_worktree_row_offers_the_sidebar_worktree_menu(
        cx: &mut gpui::TestAppContext,
    ) {
        let repo_id = RepoId(1);
        let (view, cx) = open_worktree_badge_picker(cx, repo_with_worktrees(repo_id), repo_id);

        // Row 0 is the create row, which has no menu; the worktrees follow it.
        let row = cx
            .debug_bounds("picker_prompt_item_1")
            .expect("expected a worktree row");
        let at = row.center();
        cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
        cx.simulate_event(gpui::MouseDownEvent {
            position: at,
            modifiers: gpui::Modifiers::default(),
            button: MouseButton::Right,
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_mouse_up(at, gpui::MouseButton::Right, gpui::Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        assert!(
            cx.debug_bounds("picker_row_menu").is_some(),
            "right-clicking a worktree row should open its menu"
        );
        assert!(
            cx.update(|_window, app| view.read(app).popover_host.read(app).is_open()),
            "the picker stays open underneath its row menu"
        );

        let host = cx.update(|_window, app| view.read(app).popover_host.clone());
        let (menu_labels, sidebar_labels, path) = cx.update(|_window, app| {
            host.update(app, |host, cx| {
                let entries = |model: ContextMenuModel| {
                    model
                        .entry_labels()
                        .into_iter()
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                };
                let menu = host
                    .picker_row_menu
                    .as_ref()
                    .expect("a menu should be open")
                    .clone();
                let row = worktree_badge_picker::cached(host, repo_id, "", Language::English)
                    .filtered_payloads()
                    .into_iter()
                    .find_map(|row| match row {
                        worktree_badge_picker::WorktreeBadgeRow::Worktree(path) => Some(path),
                        _ => None,
                    })
                    .expect("a worktree row");
                let branch = match &host
                    .state
                    .repos
                    .iter()
                    .find(|repo| repo.id == repo_id)
                    .expect("the repo")
                    .worktrees
                {
                    Loadable::Ready(worktrees) => worktrees
                        .iter()
                        .find(|worktree| worktree.path == row)
                        .and_then(|worktree| worktree.branch.clone()),
                    _ => None,
                };
                (
                    entries(menu.model_for_test(host, cx)),
                    entries(
                        host.context_menu_model(
                            &PopoverKind::worktree(
                                repo_id,
                                WorktreePopoverKind::Menu {
                                    path: row.clone(),
                                    branch,
                                },
                            ),
                            cx,
                        )
                        .expect("the sidebar's worktree menu"),
                    ),
                    row,
                )
            })
        });
        assert!(
            !menu_labels.is_empty(),
            "the menu for {path:?} must have entries"
        );
        assert_eq!(
            menu_labels, sidebar_labels,
            "the row menu must offer exactly what the worktree's sidebar row offers"
        );

        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        assert!(cx.debug_bounds("picker_row_menu").is_none());
        assert!(
            cx.update(|_window, app| view.read(app).popover_host.read(app).is_open()),
            "the first Escape closes the menu, not the picker"
        );
    }
}

/// Records every repository its store tries to open.
struct RecordingOpenBackend(std::sync::mpsc::Sender<PathBuf>);

impl GitBackend for RecordingOpenBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        let _ = self.0.send(workdir.to_path_buf());
        Err(Error::new(ErrorKind::Unsupported("recording backend")))
    }
}

/// Picking a worktree opens it in the invoking window even when another
/// workspace already contains it.
#[gpui::test]
fn review_regression_picking_a_worktree_open_elsewhere_uses_the_invoking_window(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let feature = std::env::temp_dir().join("ws/feature");

    let (owner_store, owner_events) = AppStore::new_test(Arc::new(TestBackend));
    let owner =
        cx.add_window(|window, cx| GitCometView::new(owner_store, owner_events, None, window, cx));
    let owner_repo = RepoId(2);
    owner
        .update(cx, |view, _window, cx| {
            crate::view::test_support::apply_state_snapshot_for_test(
                view,
                app_state_with_repo(opening_repo_state(owner_repo, &feature), owner_repo),
                cx,
            );
        })
        .expect("install the owner's repository");

    let (opened_tx, opened_rx) = std::sync::mpsc::channel();
    let (store, events) = AppStore::new_test(Arc::new(RecordingOpenBackend(opened_tx)));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                app_state_with_repo(repo_with_worktrees(repo_id), repo_id),
                cx,
            );
            this.popover_host.update(cx, |host, cx| {
                worktree_badge_picker::activate(
                    host,
                    repo_id,
                    WorktreeBadgeRow::Worktree(feature.clone()),
                    "",
                    window,
                    cx,
                );
            });
        });
    });
    cx.run_until_parked();

    assert_eq!(
        opened_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("open in the invoking window"),
        feature
    );
    let owners = cx.update(|_window, app| crate::app::windows_owning_repo_for_test(app, &feature));
    let invoking_window = cx.update(|window, _| window.window_handle().window_id());
    assert_eq!(owners.len(), 2);
    assert!(owners.contains(&owner.window_id()));
    assert!(owners.contains(&invoking_window));
}
