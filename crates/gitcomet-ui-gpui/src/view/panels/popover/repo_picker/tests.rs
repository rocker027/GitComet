use super::*;

fn row(path: &str, recency: usize) -> SortableRow {
    sortable_row(
        RepoPickerEntry::Closed(std::path::PathBuf::from(path)),
        std::path::Path::new(path),
        RECENTLY_CLOSED_SECTION,
        recency,
        true,
    )
}

fn names(rows: &[SortableRow]) -> Vec<&str> {
    rows.iter().map(|row| row.name_key.as_str()).collect()
}

fn fixture() -> Vec<SortableRow> {
    vec![
        row("/tmp/b-parent/Alpha", 0),
        row("/tmp/a-parent/zulu", 1),
        row("/tmp/c-parent/mike", 2),
    ]
}

#[test]
fn newest_and_oldest_follow_recency_rank() {
    let mut rows = fixture();
    sort_rows(&mut rows, RepoPickerSort::Newest);
    assert_eq!(names(&rows), vec!["alpha", "zulu", "mike"]);

    sort_rows(&mut rows, RepoPickerSort::Oldest);
    assert_eq!(names(&rows), vec!["mike", "zulu", "alpha"]);
}

#[test]
fn name_sort_is_case_insensitive_and_ignores_parent_directories() {
    let mut rows = fixture();
    sort_rows(&mut rows, RepoPickerSort::Name);
    assert_eq!(names(&rows), vec!["alpha", "mike", "zulu"]);
}

#[test]
fn path_sort_orders_by_full_path_not_repo_name() {
    let mut rows = fixture();
    sort_rows(&mut rows, RepoPickerSort::Path);
    assert_eq!(names(&rows), vec!["zulu", "alpha", "mike"]);
}

#[test]
fn sort_round_trips_through_its_storage_key() {
    for sort in RepoPickerSort::ALL {
        assert_eq!(
            RepoPickerSort::from_storage_key(sort.storage_key()),
            Some(sort)
        );
    }
    assert_eq!(RepoPickerSort::from_storage_key("nonsense"), None);
}

#[test]
fn sort_toggle_includes_the_selected_sort() {
    let english = Language::English;
    let chinese = Language::TraditionalChinese;
    assert_eq!(
        sort_toggle_label(RepoPickerSort::Newest, english),
        "Sort: Newest"
    );
    assert_eq!(
        sort_toggle_label(RepoPickerSort::Oldest, english),
        "Sort: Oldest"
    );
    assert_eq!(
        sort_toggle_label(RepoPickerSort::Name, english),
        format!("Sort: {}", RepoPickerSort::Name.label(english))
    );
    assert_eq!(
        sort_toggle_label(RepoPickerSort::Path, chinese),
        "排序：路徑（A–Z）"
    );
}

#[test]
fn review_regression_selected_sort_applies_to_workspace_rows() {
    let mut zulu = session::Workspace::new(vec!["/repos/zulu".into()]);
    zulu.custom_name = Some("Zulu".to_string());
    zulu.last_activation_order = 20;
    let mut alpha = session::Workspace::new(vec!["/repos/alpha".into()]);
    alpha.custom_name = Some("Alpha".to_string());
    alpha.last_activation_order = 10;

    let rows = workspace_rows(vec![zulu, alpha], RepoPickerSort::Name, Language::English);

    assert_eq!(names(&rows), vec!["alpha", "zulu"]);
}

#[test]
fn traditional_chinese_workspace_rows_keep_english_search_and_canonical_sections() {
    let mut workspace = session::Workspace::new(vec!["/repos/產品".into()]);
    workspace.custom_name = Some("Release".into());
    workspace.restore_on_launch = true;
    let row = workspace_row(workspace, 0, Language::TraditionalChinese);
    let items = [row.item];

    assert_eq!(
        items[0].debug_display_text(),
        "Release - 已開啟 · 1 個儲存庫"
    );
    for query in ["release", "產品", "1 個儲存庫"] {
        let layout = components::picker_prompt_layout(&items, query);
        assert_eq!(layout.item_indices, vec![0], "{query}");
        assert!(layout.match_ranges[0].is_some(), "{query}");
        assert_eq!(layout.headers[0].1.label.as_ref(), WORKSPACES_SECTION);
    }

    let alias_layout = components::picker_prompt_layout(&items, "Open · 1 repository");
    assert_eq!(alias_layout.item_indices, vec![0]);
    assert_eq!(alias_layout.match_ranges, vec![None]);
    assert_eq!(
        section_storage_key(WORKSPACES_SECTION),
        Some("window_groups")
    );
}
