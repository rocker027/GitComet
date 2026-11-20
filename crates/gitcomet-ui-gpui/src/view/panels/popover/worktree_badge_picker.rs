use super::*;
use crate::localization::Language;
use std::hash::Hash;
use std::rc::Rc;

/// What activating a row in the workspace picker does.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WorktreeBadgeRow {
    /// Open (or re-activate) the worktree at this path.
    Worktree(std::path::PathBuf),
    /// Hand off to the Add-worktree dialog, prefilled from the query.
    CreateNew,
    /// The nth entry of the row menu floating over the picker, which takes the
    /// arrow keys while it is up.
    RowAction(usize),
}

/// Rows the picker displays, in render order, paired with what each one does
/// and which row carries the "current worktree" check.
///
/// Both the panel and keyboard navigation go through this, so the list the user
/// sees and the list Enter walks can never disagree.
pub(super) struct WorktreeBadgeRows {
    pub(super) items: Vec<components::PickerPromptItem>,
    pub(super) rows: Vec<WorktreeBadgeRow>,
    /// Index of the active worktree **before filtering** — `PickerPrompt`
    /// compares `marked_index` against the pre-filter index.
    pub(super) marked_index: Option<usize>,
}

/// Ref the create row bases a new worktree on: the current branch, or `HEAD`
/// when detached or not yet loaded.
fn create_base_ref(repo: &RepoState) -> String {
    match &repo.head_branch {
        Loadable::Ready(head) if head != "HEAD" => head.clone(),
        _ => "HEAD".to_string(),
    }
}

/// Takes the repository rather than the host so the result is a pure function of
/// its inputs, which is what lets [`rows_cache`](super::rows_cache) memoise it
/// across frames.
#[cfg(feature = "benchmarks")]
pub(super) fn rows(repo: &RepoState, query: &str) -> WorktreeBadgeRows {
    rows_for_language(repo, query, Language::English)
}

fn rows_for_language(repo: &RepoState, query: &str, language: Language) -> WorktreeBadgeRows {
    let capacity = repo
        .worktrees
        .ready()
        .map_or(0usize, |values| values.len())
        .saturating_add(1);
    let mut items = Vec::with_capacity(capacity);
    let mut rows = Vec::with_capacity(capacity);
    let mut marked_index = None;

    // Create row first, mirroring the placeholder's "select or type to create".
    // Its searchable text is the query itself, so it survives any filter —
    // `match_items` drops rows whose match text does not contain the query.
    let base = create_base_ref(repo);
    let query = query.trim();
    // The base ref goes on the detail line rather than trailing the title, so this
    // row is the same height as the worktree rows below it and the list reads as one
    // block instead of a short row stacked on tall ones.
    let create_item = if query.is_empty() {
        components::PickerPromptItem::from_parts([components::PickerPromptItemPart::new(
            language.text("Create new worktree", "新增工作樹"),
        )
        .flexible(false)
        .searchable(false)
        .tooltip(false)])
    } else {
        components::PickerPromptItem::from_parts([
            components::PickerPromptItemPart::new(language.text("Create worktree ", "新增工作樹 "))
                .flexible(false)
                .searchable(false)
                .tooltip(false),
            components::PickerPromptItemPart::new(query.to_string()).flexible(false),
        ])
    };
    let based_on = match language {
        Language::English => format!("Based off {base}"),
        Language::TraditionalChinese => format!("起始參照 {base}"),
    };
    items.push(
        create_item
            .secondary_parts([components::PickerPromptItemPart::new(based_on).searchable(false)])
            .icon("icons/plus.svg"),
    );
    rows.push(WorktreeBadgeRow::CreateNew);

    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return WorktreeBadgeRows {
            items,
            rows,
            marked_index,
        };
    };

    for worktree in worktrees.iter() {
        // Title line: the folder the worktree lives in, then what is checked out
        // in it — the two things that identify a worktree at a glance.
        let name = crate::view::path_display::repo_path_name(&worktree.path);
        let mut primary = Vec::with_capacity(3);
        primary.push(
            components::PickerPromptItemPart::new(name.to_string())
                .profile(components::TextTruncationProfile::End)
                .flexible(false),
        );

        if let Some(branch) = &worktree.branch {
            primary.push(components::PickerPromptItemPart::separator(
                language.text("  on  ", "  ·  "),
            ));
            primary.push(
                components::PickerPromptItemPart::new(branch.clone())
                    .profile(components::TextTruncationProfile::End)
                    .flexible(false),
            );
        } else if worktree.detached {
            primary.push(components::PickerPromptItemPart::separator("  "));
            primary.push(
                components::PickerPromptItemPart::new(language.text("detached", "分離 HEAD"))
                    .flexible(false)
                    .searchable(false)
                    .dim()
                    .tooltip(false),
            );
        }

        // Detail line: where it is on disk, and which commit it sits on. The path
        // stays searchable so a path query still finds its row.
        let mut secondary = Vec::with_capacity(3);
        if let Some(primary) = worktrees.first()
            && primary.path != worktree.path
        {
            secondary.push(
                components::PickerPromptItemPart::new(format!(
                    "{} / ",
                    crate::view::path_display::repo_path_name(&primary.path)
                ))
                .flexible(false),
            );
        }
        if let Some(head) = &worktree.head {
            let sha = head.as_ref();
            secondary.push(
                components::PickerPromptItemPart::new(sha.get(0..8).unwrap_or(sha).to_string())
                    .flexible(false)
                    .searchable(false)
                    // Eight characters wide and never squeezed: nothing to reveal.
                    .tooltip(false),
            );
            secondary.push(components::PickerPromptItemPart::separator("  •  "));
        }
        secondary.push(components::PickerPromptItemPart::path(
            worktree.path.display().to_string(),
        ));

        if worktree.path == repo.spec.workdir {
            marked_index = Some(items.len());
        }
        items.push(
            components::PickerPromptItem::from_parts(primary)
                .secondary_parts(secondary)
                .icon("icons/git_worktree.svg"),
        );
        rows.push(WorktreeBadgeRow::Worktree(worktree.path.clone()));
    }

    WorktreeBadgeRows {
        items,
        rows,
        marked_index,
    }
}

/// Everything [`rows_for_language`] reads, digested for the cache. Mirrors the
/// `RepoPopoverKind::Worktree` arm of [`super::fingerprint`], which tracks HEAD
/// as well because the create row names the ref it would branch from. No
/// relative dates on these rows, so the clock stays out of it: they say what is
/// checked out and where, not when.
pub(super) fn rows_signature(repo: &RepoState, language: Language) -> u64 {
    super::rows_cache::signature(|hasher| {
        language.key().hash(hasher);
        repo.id.hash(hasher);
        repo.head_branch_rev.hash(hasher);
        repo.worktrees_rev.hash(hasher);
        super::rows_cache::loadable_kind(&repo.open).hash(hasher);
        // The picker marks the row whose path is the active workdir.
        repo.spec.workdir.hash(hasher);
    })
}

/// The cached rows for `query`: the items, the filtered layout, and the payload
/// behind each row. Every caller goes through this, so a frame that only
/// repaints reuses the rows instead of rebuilding them.
pub(super) fn cached(
    this: &PopoverHost,
    repo_id: RepoId,
    query: &str,
    language: Language,
) -> Rc<super::rows_cache::CachedRows<WorktreeBadgeRow>> {
    let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
        return super::rows_cache::CachedRows::empty();
    };
    let key = super::rows_cache::RowsCacheKey::new(
        super::rows_cache::RowsCacheOwner::Workspace,
        rows_signature(repo, language),
        query,
    )
    .with_query_dependent_model();
    super::rows_cache::get_or_build(&this.worktree_badge_picker_rows_cache, key, |_now| {
        let built = rows_for_language(repo, query, language);
        (built.items, built.rows, built.marked_index)
    })
}

/// Payloads for the rows surviving `query`, in the order the picker renders them
/// — the list keyboard navigation walks.
pub(super) fn nav_targets(
    this: &PopoverHost,
    repo_id: RepoId,
    query: &str,
    language: Language,
) -> Vec<WorktreeBadgeRow> {
    cached(this, repo_id, query, language).filtered_payloads()
}

/// Activates a row: opens the worktree, or hands the query to the Add dialog.
pub(super) fn activate(
    this: &mut PopoverHost,
    repo_id: RepoId,
    row: WorktreeBadgeRow,
    query: &str,
    window: &mut Window,
    cx: &mut gpui::Context<PopoverHost>,
) {
    match row {
        // Menu entries never reach here: they run through the row menu itself.
        WorktreeBadgeRow::RowAction(_) => {}
        WorktreeBadgeRow::Worktree(path) => {
            let is_current = this
                .state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .is_some_and(|repo| repo.spec.workdir == path);
            // Re-opening the active worktree would only re-activate its own
            // tab; just dismiss instead.
            if !is_current {
                crate::app::open_repository_from_view(cx, window.window_handle().window_id(), path);
            }
            this.close_popover(cx);
        }
        WorktreeBadgeRow::CreateNew => {
            let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
                return;
            };
            // A new branch off the base the row's "based on <head>" label names.
            this.pending_worktree_add_prefill = Some(WorktreeAddPrefill {
                source: create_base_ref(repo),
                create_branch: true,
                new_branch: query.trim().to_string(),
                open_after: false,
            });
            this.open_popover_centered(
                PopoverKind::worktree(repo_id, WorktreePopoverKind::AddPrompt),
                window,
                cx,
            );
        }
    }
}

pub(super) fn search_placeholder(language: Language) -> &'static str {
    language.text(
        "Select or type to create a worktree",
        "選擇工作樹或輸入名稱以新增",
    )
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let ui_scale = super::popover_ui_scale(cx);
    let ui_scale_percent = ui_scale.percent();
    let scaled_px = crate::ui_scale::scaler(ui_scale_percent);
    let width = super::LARGE_PICKER_WIDTH;

    let Some(search) = this.worktree_badge_picker_search_input.clone() else {
        return components::context_menu_label(
            theme,
            ui_scale_percent,
            language.text("Search input not initialized", "搜尋欄尚未初始化"),
            Some(this.tooltip_host.clone()),
            cx,
        );
    };

    let placeholder = search_placeholder(language);
    if search.read(cx).placeholder() != placeholder {
        search.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
    }
    // Read the query from the same input `PickerPrompt::render` filters with,
    // so the rows built here match the rows it displays.
    let query = search.read_with(cx, |input, _| input.text().trim().to_string());
    let built = cached(this, repo_id, &query, language);
    let row_payloads = Rc::clone(&built.payloads);
    let menu_payloads = Rc::clone(&built.payloads);

    components::context_menu(
        theme,
        components::PickerPrompt::new(search, this.picker_prompt_scroll.clone())
            // Prebuilt items and layout: the cache already filtered and sorted
            // them, so `render` must not repeat that work.
            .prebuilt_items(Rc::clone(&built.items), Rc::clone(&built.layout))
            // Long ref lists render only what is on screen; keyboard
            // navigation scrolls by the row geometry to match
            .tooltip_host(this.tooltip_host.clone())
            // Only reachable when the repo is gone: the create row always
            // matches, so a present repo never yields an empty list.
            .empty_text(language.text("No repository", "沒有儲存庫"))
            .max_height(scaled_px(components::PICKER_LIST_MAX_HEIGHT_PX))
            .selected_index(
                // While a row menu is open the arrow keys walk its entries, so
                // the list's highlight marks the invoking row instead.
                this.picker_row_menu
                    .as_ref()
                    .map(|menu| menu.display_index)
                    .or(this.worktree_badge_picker_selected_index),
            )
            .marked_index(built.marked_index)
            // Right-click offers the worktree its sidebar row offers, floating
            // over the picker rather than replacing it.
            .on_context_menu(cx.listener(
                move |this, event: &components::PickerPromptContextMenuEvent, _window, cx| {
                    let Some(row) = menu_payloads.get(event.original_index).cloned() else {
                        return;
                    };
                    let target = picker_row_menu::PickerRowMenuTarget::Worktree { repo_id, row };
                    if !target.has_menu(this) {
                        return;
                    }
                    picker_row_menu::open(this, target, event.display_index, event.position, cx);
                },
            ))
            .render(
                theme,
                ui_scale_percent,
                cx,
                move |this, ix, _e, window, cx| {
                    let Some(row) = row_payloads.get(ix).cloned() else {
                        return;
                    };
                    let query = this
                        .worktree_badge_picker_search_input
                        .as_ref()
                        .map(|input| input.read(cx).text().trim().to_string())
                        .unwrap_or_default();
                    activate(this, repo_id, row, &query, window, cx);
                },
            ),
    )
    .w(width.preferred_px(ui_scale))
}
