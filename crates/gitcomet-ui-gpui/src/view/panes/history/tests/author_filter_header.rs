use super::*;
use crate::localization::Language;

#[gpui::test]
fn author_filter_moves_to_message_header_on_resize_and_clears_through_its_picker(
    cx: &mut gpui::TestAppContext,
) {
    assert_responsive_author_filter(cx, Language::English);
}

#[gpui::test]
fn chinese_author_filter_keeps_its_full_tooltip_when_the_author_column_hides(
    cx: &mut gpui::TestAppContext,
) {
    assert_responsive_author_filter(cx, Language::TraditionalChinese);
}

#[gpui::test]
fn narrow_author_filter_keeps_history_retry_visible_and_clickable(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::services::HistorySnapshot;

    cx.update(crate::appearance::pin_compact_for_test);
    let author = "An author with a very long name competing with the history retry action";
    let mut tip = commit("tip", &[], "Filtered commit");
    tip.author = author.into();
    let (view, cx, mut state, store) = refresh::mount(cx, Arc::new(log_page(vec![tip], None)));
    cx.simulate_resize(size(px(900.0), px(900.0)));
    let repo = &mut state.repos[0];
    repo.history_state.history_author_filter = Some(author.into());
    repo.history_state.indexed.error = Some("failed to read complete history".into());
    repo.history_state.indexed.status_rev += 1;
    // Retry first clears this failed request. With no log snapshot the fixture
    // stops there, proving the actual link dispatched Retry without starting I/O.
    repo.history_state.log_snapshot = None;
    repo.history_state.indexed.requested = Some(HistorySnapshot("failed-request".into()));
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    cx.update(|window, app| {
        app.set_global(Language::English);
        window.refresh();
    });
    wait_until(cx, "narrow history retry beside the author filter", |cx| {
        cx.debug_bounds("history_author_header_cell").is_none()
            && cx.debug_bounds("history_index_status").is_some()
            && cx.debug_bounds("history_filter_toolbar").is_some()
    });
    let toolbar = cx.debug_bounds("history_filter_toolbar").unwrap();
    let status = cx.debug_bounds("history_index_status").unwrap();
    let chip = cx.debug_bounds("history_author_filter_header").unwrap();
    assert!(
        status.size.width >= px(50.0),
        "retry needs a usable pointer target"
    );
    assert!(status.size.height >= px(12.0));
    assert!(status.left() >= toolbar.left() && status.right() <= toolbar.right());
    assert!(chip.left() >= status.right() && chip.right() <= toolbar.right());
    assert!(
        chip.size.width > px(0.0),
        "the active author must remain discoverable"
    );
    cx.simulate_mouse_move(status.center(), None, gpui::Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view).as_deref(),
        Some("Full history unavailable · retry")
    );
    assert!(
        store.snapshot().repos[0]
            .history_state
            .indexed
            .requested
            .is_some()
    );
    cx.simulate_click(status.center(), gpui::Modifiers::default());
    wait_until(cx, "original history retry action dispatched", |_| {
        store.snapshot().repos[0]
            .history_state
            .indexed
            .requested
            .is_none()
    });
    assert_eq!(
        store.snapshot().repos[0]
            .history_state
            .history_author_filter
            .as_deref(),
        Some(author)
    );
    cx.update(|_, app| {
        assert!(crate::view::test_support::popover_kind(view.read(app), app).is_none());
    });
}

fn assert_responsive_author_filter(cx: &mut gpui::TestAppContext, language: Language) {
    cx.update(crate::appearance::pin_compact_for_test);
    let author = "A very long author name that must remain identifiable after resizing";
    let mut tip = commit("tip", &[], "Filtered commit");
    tip.author = author.into();
    let (view, cx, mut state, store) = refresh::mount(cx, Arc::new(log_page(vec![tip], None)));
    cx.simulate_resize(size(px(1800.0), px(900.0)));
    state.repos[0].history_state.history_author_filter = Some(author.into());
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    ensure_history_cache_for_tests(cx, &view, Arc::new(state));
    cx.update(|window, app| {
        app.set_global(language);
        window.refresh();
    });
    wait_until(cx, "wide author header", |cx| {
        cx.debug_bounds("history_author_header_cell").is_some()
            && cx.debug_bounds("history_author_filter_header").is_some()
    });
    let wide_cell = cx.debug_bounds("history_author_header_cell").unwrap();
    let wide_chip = cx.debug_bounds("history_author_filter_header").unwrap();
    assert!(wide_chip.left() >= wide_cell.left());
    assert!(
        cx.debug_bounds("history_author_filter_message_header")
            .is_none(),
        "wide layout must not duplicate the author control in MESSAGE"
    );
    assert!(cx.debug_bounds("history_col_resize_author").is_some());

    // Find an actual responsive width where author is hidden but MESSAGE still
    // has its existing minimum budget. Side-pane defaults affect the threshold.
    let mut normal_narrow = false;
    for width in (900..1800).step_by(40).rev() {
        cx.simulate_resize(size(px(width as f32), px(900.0)));
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update(|window, app| {
                window.refresh();
                let _ = window.draw(app);
            });
        }
        if cx
            .debug_bounds("history_author_filter_message_header")
            .is_some()
        {
            normal_narrow = true;
            break;
        }
    }
    assert!(
        normal_narrow,
        "a normal narrow layout must keep the filter in MESSAGE"
    );
    assert!(cx.debug_bounds("history_filter_toolbar").is_none());
    let message = cx.debug_bounds("history_message_header_cell").unwrap();
    let normal_chip = cx.debug_bounds("history_author_filter_header").unwrap();
    assert!(normal_chip.left() > message.left() && normal_chip.right() <= message.right());

    cx.simulate_resize(size(px(900.0), px(900.0)));
    wait_until(cx, "extremely narrow author filter toolbar", |cx| {
        cx.debug_bounds("history_author_header_cell").is_none()
            && cx.debug_bounds("history_filter_toolbar").is_some()
    });
    assert!(
        cx.debug_bounds("history_author_filter_message_header")
            .is_none()
    );
    let toolbar = cx.debug_bounds("history_filter_toolbar").unwrap();
    let message = cx.debug_bounds("history_message_header_cell").unwrap();
    let narrow_chip = cx.debug_bounds("history_author_filter_header").unwrap();
    let chip_container = cx
        .debug_bounds("history_author_filter_toolbar_control")
        .unwrap();
    assert!(
        narrow_chip.top() >= message.bottom(),
        "the fallback must sit below the unmodified MESSAGE header"
    );
    assert!(narrow_chip.left() >= toolbar.left() && narrow_chip.right() <= toolbar.right());
    assert!(
        narrow_chip.left() >= chip_container.left()
            && narrow_chip.right() <= chip_container.right(),
        "the clickable chip itself must fit its allocated flex width, before clipping"
    );
    assert!(narrow_chip.size.width > px(0.0) && narrow_chip.size.width <= px(160.0));
    assert!(cx.debug_bounds("history_col_resize_author").is_none());
    assert_eq!(
        store.snapshot().repos[0]
            .history_state
            .history_author_filter
            .as_deref(),
        Some(author)
    );

    cx.simulate_mouse_move(narrow_chip.center(), None, gpui::Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    let tooltip = match language {
        Language::English => format!("Author filter: {author}"),
        Language::TraditionalChinese => format!("作者篩選：{author}"),
    };
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view).as_deref(),
        Some(tooltip.as_str())
    );

    cx.simulate_click(narrow_chip.center(), gpui::Modifiers::default());
    wait_until(cx, "original author picker opened", |cx| {
        cx.update(|_, app| {
            matches!(
                crate::view::test_support::popover_kind(view.read(app), app),
                Some(PopoverKind::HistoryAuthorFilter { repo_id: RepoId(1) })
            )
        })
    });
    // Activate the actual unfiltered row once the picker has been laid out.
    wait_until(cx, "All authors row", |cx| {
        cx.debug_bounds("picker_prompt_item_0").is_some()
    });
    let all_authors = cx.debug_bounds("picker_prompt_item_0").unwrap();
    cx.simulate_click(all_authors.center(), gpui::Modifiers::default());
    wait_for_store(cx, &view, "author filter cleared", || {
        store.snapshot().repos[0]
            .history_state
            .history_author_filter
            .is_none()
    });
    wait_until(cx, "narrow unfiltered header has no author control", |cx| {
        cx.debug_bounds("history_author_filter_header").is_none()
            && cx
                .debug_bounds("history_author_filter_message_header")
                .is_none()
    });
    assert!(cx.debug_bounds("history_filter_toolbar").is_none());
    cx.update(|_, app| {
        assert!(crate::view::test_support::popover_kind(view.read(app), app).is_none())
    });

    cx.simulate_resize(size(px(1800.0), px(900.0)));
    wait_until(cx, "wide unfiltered author control restored", |cx| {
        cx.debug_bounds("history_author_header_cell").is_some()
            && cx.debug_bounds("history_author_filter_header").is_some()
    });
    assert!(
        cx.debug_bounds("history_author_filter_message_header")
            .is_none()
    );
}
