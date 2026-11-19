use super::branch::{TrackingRepo, create_tracking_store, wait_until};
use super::*;

#[gpui::test]
fn stash_menu_has_apply_pop_and_drop_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(8);
    let index = 3usize;
    let message = "WIP".to_string();

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::StashMenu {
                            repo_id,
                            index,
                            message: message.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected stash context menu model");

        let apply_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Apply stash" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            apply_action,
            Some(ContextMenuAction::ApplyStash {
                repo_id: rid,
                index: ix
            }) if rid == repo_id && ix == index
        ));

        let pop_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Pop stash" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            pop_action,
            Some(ContextMenuAction::PopStash {
                repo_id: rid,
                index: ix
            }) if rid == repo_id && ix == index
        ));

        let drop_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Drop stash…" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            drop_action,
            Some(ContextMenuAction::DropStashConfirm {
                repo_id: rid,
                index: ix,
                message: ref msg
            }) if rid == repo_id && ix == index && msg == &message
        ));
    });
}

#[gpui::test]
fn stash_menu_drop_action_opens_drop_confirm_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(9);
    let index = 1usize;
    let message = "Drop me".to_string();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.popover_anchor = Some(PopoverAnchor::Point(point(px(32.0), px(48.0))));
                host.context_menu_activate_action(
                    ContextMenuAction::DropStashConfirm {
                        repo_id,
                        index,
                        message: message.clone(),
                    },
                    window,
                    cx,
                );

                match host.popover.as_ref() {
                    Some(PopoverKind::StashDropConfirm {
                        repo_id: rid,
                        index: ix,
                        message: msg,
                    }) => {
                        assert_eq!(*rid, repo_id);
                        assert_eq!(*ix, index);
                        assert_eq!(msg, &message);
                    }
                    _ => panic!("expected stash drop confirm popover to open"),
                }
            });
        });
    });
}

#[gpui::test]
fn stash_prompt_escape_cancels(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("stash-escape");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::StashPrompt.invoked_by("stash_btn".into()),
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, Some("stash_btn"));
    });

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Escape to close stash popover");
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, None);
    });
    cx.update(|window, app| {
        let root = view.read(app);
        let main_focus = root
            .popover_host
            .read(app)
            .diff_renderer
            .read(app)
            .diff_panel_focus_handle
            .clone();
        assert!(
            main_focus.is_focused(window),
            "expected Escape to move focus away from the Stash button"
        );
    });
    assert!(
        repo.actions().is_empty(),
        "expected Escape to cancel without creating a stash"
    );
}

#[gpui::test]
fn stash_prompt_renders_shortcut_hints_and_separators(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("stash-shortcuts");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::StashPrompt,
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.debug_bounds("stash_cancel_hint")
        .expect("expected stash Cancel shortcut hint");
    cx.debug_bounds("stash_go_hint")
        .expect("expected stash Create shortcut hint");
    cx.debug_bounds("stash_cancel_end_slot_separator")
        .expect("expected stash Cancel shortcut separator");
    cx.debug_bounds("stash_go_end_slot_separator")
        .expect("expected stash Create shortcut separator");
}

#[gpui::test]
fn stash_prompt_enter_stashes_and_closes(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("stash-enter");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::StashPrompt.invoked_by("stash_btn".into()),
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, Some("stash_btn"));
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.stash_message_input
                    .update(cx, |input, cx| input.set_text("wip", cx));
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Enter to close stash popover");
    cx.update(|_window, app| {
        let active_invoker = view
            .read(app)
            .active_context_menu_invoker
            .as_ref()
            .map(|id| id.as_ref());
        assert_eq!(active_invoker, None);
    });

    wait_until("stash action", || {
        repo.actions() == vec!["stash:wip:true".to_string()]
    });
}

/// Opens the stash dialog over a tracked repository and returns its view.
fn open_stash_prompt<'a>(
    cx: &'a mut gpui::TestAppContext,
    label: &str,
) -> (
    gpui::Entity<GitCometView>,
    &'a mut gpui::VisualTestContext,
    Arc<TrackingRepo>,
) {
    let (store, events, repo, _workdir) = create_tracking_store(label);
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        let _ = window.draw(app);
    });
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::StashPrompt.invoked_by("stash_btn".into()),
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    (view, cx, repo)
}

/// The message is optional, as in GitKraken: Git names the stash itself.
#[gpui::test]
fn stash_prompt_enter_with_empty_input_stashes_without_a_message(cx: &mut gpui::TestAppContext) {
    let (view, cx, repo) = open_stash_prompt(cx, "stash-empty-enter");

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Enter to stash with an empty message");
    wait_until("stash action", || {
        repo.actions() == vec!["stash::true".to_string()]
    });
}

#[gpui::test]
fn stash_prompt_can_leave_untracked_files_out(cx: &mut gpui::TestAppContext) {
    let (view, cx, repo) = open_stash_prompt(cx, "stash-tracked-only");

    let toggle = cx
        .debug_bounds("stash_include_untracked_toggle")
        .expect("expected the include-untracked toggle")
        .center();
    cx.simulate_click(toggle, gpui::Modifiers::default());
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        !cx.update(|_window, app| view
            .read(app)
            .popover_host
            .read(app)
            .stash_include_untracked),
        "expected the toggle to turn untracked files off"
    );

    // Back to the message field, where Enter submits.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let focus = host.stash_message_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    wait_until("stash action", || {
        repo.actions() == vec!["stash::false".to_string()]
    });
}

/// The toolbar Pop restores the newest stash in one click, as GitKraken's.
#[gpui::test]
fn the_toolbar_pop_button_pops_the_newest_stash(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("stash-toolbar-pop");
    let store_for_view = store.clone();
    let repo_id = store
        .snapshot()
        .active_repo
        .expect("the tracked repository is active");
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    store.dispatch(Msg::Internal(
        gitcomet_state::msg::InternalMsg::StashesLoaded {
            repo_id,
            result: Ok(vec![gitcomet_core::domain::StashEntry {
                index: 0,
                id: CommitId("0123456789012345678901234567890123456789".into()),
                message: "WIP on main".into(),
                created_at: None,
            }]),
        },
    ));
    wait_until("stash listed", || {
        store.snapshot().repos.iter().any(|r| {
            r.id == repo_id && matches!(&r.stashes, Loadable::Ready(list) if list.len() == 1)
        })
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let pop = cx
        .debug_bounds("stash_pop")
        .expect("expected the Pop button beside Stash")
        .center();
    cx.simulate_click(pop, gpui::Modifiers::default());
    cx.run_until_parked();

    wait_until("stash popped", || {
        repo.actions() == vec!["stash-apply:0".to_string(), "stash-drop:0".to_string()]
    });
}

/// A stash's node in the history graph opens the stash menu, whose Drop goes
/// through the existing confirmation.
#[gpui::test]
fn a_stash_node_in_the_graph_opens_the_stash_menu(cx: &mut gpui::TestAppContext) {
    use crate::view::panels::tests::{app_state_with_repo, push_test_state};
    use gitcomet_state::model::RepoState;

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(41);
    let stash_id = CommitId("5555555555555555555555555555555555555555".into());
    let mut repo = RepoState::new_opening(
        repo_id,
        gitcomet_core::domain::RepoSpec {
            workdir: std::env::temp_dir().join("gitcomet_ui_test_stash_node_menu"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::WorkingTree);
    repo.stashes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::StashEntry {
        index: 2,
        id: stash_id.clone(),
        message: "WIP on main".into(),
        created_at: None,
    }]));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    let mut menu = |commit_id: CommitId, refs: crate::view::CommitMenuRefs| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::CommitMenu {
                            repo_id,
                            commit_id,
                            refs,
                        },
                        cx,
                    )
                    .expect("expected a menu")
                })
            })
        })
    };
    let stash_menu = menu(stash_id.clone(), crate::view::CommitMenuRefs::All);
    let actions: Vec<ContextMenuAction> = stash_menu
        .items
        .iter()
        .filter_map(|item| match item {
            ContextMenuItem::Entry { action, .. } => Some((**action).clone()),
            _ => None,
        })
        .collect();
    assert!(actions.iter().any(|action| matches!(
        action,
        ContextMenuAction::PopStash { repo_id: r, index: 2 } if *r == repo_id
    )));
    // Drop asks first: the entry opens the confirmation, never drops.
    assert!(
        actions
            .iter()
            .any(|action| matches!(action, ContextMenuAction::DropStashConfirm { index: 2, .. }))
    );

    // Any other commit keeps the commit menu.
    let other = menu(
        CommitId("6666666666666666666666666666666666666666".into()),
        crate::view::CommitMenuRefs::All,
    );
    assert!(!other.items.iter().any(|item| matches!(
        item,
        ContextMenuItem::Entry { action, .. }
            if matches!(action.as_ref(), ContextMenuAction::PopStash { .. })
    )));
}
