//! Multi-selection and range comparison views: ordered commit cards and the
//! changed files between two points.

use super::*;
use crate::localization::{Language, text};
use crate::view::panes::{ComparisonCardCache, ComparisonOrderCache};
use crate::view::rows::CommitCard;
use gitcomet_state::model::{ComparisonSource, RangeSelection};
use rustc_hash::FxHashSet;

const MULTI_COMMIT_ROW_HEIGHT_PX: f32 = 44.0;

/// How much of the comparison body the compared-commit cards may fill before
/// they start scrolling instead of growing. A range comparison has two
/// endpoints, but a multi-selection comparison has one card per selected
/// commit, and an unbounded column of those would push the changed-file list —
/// the part the user actually came for — off the bottom of the pane. At half,
/// the two lists split the body evenly once the selection is large enough to
/// need it, whatever height the pane happens to have.
const COMPARISON_CARDS_MAX_BODY_FRACTION: f32 = 0.5;

/// Floor for the comparison's changed-file section — a label row, the filter
/// tabs, and a row or two of list. Keeps the capped card block above it from
/// claiming the whole pane when the pane is shorter than the card cap allows.
pub(super) const RANGE_FILES_SECTION_MIN_HEIGHT_PX: f32 = 70.0;

impl DetailsPaneView {
    /// Whether the pane compares commits (a range, or a multi-selection): the
    /// one view that shows loaded history rows.
    pub(in crate::view) fn shows_comparison(repo: &RepoState) -> bool {
        repo.history_state.range_selection.is_some()
            || repo.history_state.multi_selection.is_multi()
    }

    /// Selected IDs in displayed history order, independent of the bounded
    /// metadata cache. Missing metadata must not shrink the selection's cards.
    pub(super) fn multi_selected_commit_ids_in_log_order(repo: &RepoState) -> Vec<CommitId> {
        let selection = &repo.history_state.multi_selection;
        let indexed = &repo.history_state.indexed;
        if let Some(index) = indexed.displayed_index.as_ref().or(indexed.index.as_ref()) {
            let mut selected = selection.commits.as_ref().clone();
            selected.sort_by_cached_key(|id| index.position(id.as_ref()).unwrap_or(usize::MAX));
            return selected;
        }
        let Loadable::Ready(page) = &repo.log else {
            return Vec::new();
        };
        // Hash the selection first: this runs per frame (twice, and once more
        // per visible row batch) over the whole loaded page, and
        // `CommitMultiSelection::contains` is a linear scan — so a large
        // selection against a large page would be quadratic on every repaint.
        let selected: FxHashSet<&CommitId> = selection.commits.iter().collect();
        page.commits
            .iter()
            .filter(|commit| selected.contains(&commit.id))
            .map(|commit| commit.id.clone())
            .collect()
    }

    /// The comparison whose endpoints are the cards; `None` when the cards are
    /// the selected commits (a merged diff, or a list with no comparison).
    fn endpoint_range(repo: &RepoState) -> Option<&RangeSelection> {
        let range = repo.history_state.range_selection.as_ref()?;
        match range.source {
            ComparisonSource::MergedSelection => None,
            ComparisonSource::SelectedPair
            | ComparisonSource::AdjacentPair
            | ComparisonSource::Explicit => Some(range),
        }
    }

    /// Commit ids for the cards, newest first. Endpoint cards skip the empty
    /// tree, so a working-tree comparison yields a single card.
    pub(super) fn range_comparison_commit_ids(repo: &RepoState) -> Vec<CommitId> {
        let Some(range) = Self::endpoint_range(repo) else {
            return Self::multi_selected_commit_ids_in_log_order(repo);
        };
        range
            .to
            .iter()
            .chain(std::iter::once(&range.from))
            .filter(|id| !gitcomet_core::domain::is_empty_tree_id(id.as_ref()))
            .cloned()
            .collect()
    }

    pub(super) fn comparison_commits<'a>(
        repo: &'a RepoState,
        ids: &[CommitId],
    ) -> Vec<Option<&'a Commit>> {
        let indexed = &repo.history_state.indexed;
        let page = match &repo.log {
            Loadable::Ready(page) => Some(page),
            _ => repo.history_state.retained_log_while_loading.as_ref(),
        };
        // Hash the fallback page once, including for legacy histories where it
        // may contain thousands of commits. Never scan it per selected ID.
        let bootstrap: FxHashMap<_, _> = page
            .into_iter()
            .flat_map(|page| &page.commits)
            .map(|commit| (&commit.id, commit))
            .collect();
        // Resolve by immutable ID against the cache's own index: during handoff
        // its row numbers can differ from those of the displayed presentation.
        ids.iter()
            .map(|id| {
                indexed
                    .range_index
                    .as_ref()
                    .and_then(|index| indexed.commit(&index.snapshot, index.position(id.as_ref())?))
                    .or_else(|| bootstrap.get(id).copied())
            })
            .collect()
    }

    fn comparison_order_key(repo: &RepoState) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut key = rustc_hash::FxHasher::default();
        repo.id.hash(&mut key);
        repo.log_rev.hash(&mut key);
        if let Some(range) = Self::endpoint_range(repo) {
            range.from.hash(&mut key);
            range.to.hash(&mut key);
        }
        (Arc::as_ptr(&repo.history_state.multi_selection.commits) as usize).hash(&mut key);
        repo.history_state
            .indexed
            .displayed_index
            .as_ref()
            .or(repo.history_state.indexed.index.as_ref())
            .map(|index| Arc::as_ptr(index) as usize)
            .hash(&mut key);
        key.finish()
    }

    pub(super) fn ensure_comparison_order(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo) = self.active_repo() else {
            return;
        };
        let key = Self::comparison_order_key(repo);
        if self
            .comparison_order
            .as_ref()
            .is_some_and(|cache| cache.key == key)
            || self.comparison_order_pending == Some(key)
        {
            return;
        }
        // Small endpoint comparisons are ready in their first frame. Large
        // selections are sorted on the executor and published by generation.
        if Self::comparison_count(repo) <= 256 {
            let ordered = Arc::new(Self::range_comparison_commit_ids(repo));
            self.comparison_order = Some(Self::comparison_order_cache(repo, key, ordered));
            self.comparison_order_pending = None;
            return;
        }
        let source = Self::comparison_order_cache(repo, key, Arc::new(Vec::new()));
        let repo = repo.clone();
        self.comparison_order_pending = Some(key);
        cx.spawn(async move |view, cx| {
            let ordered = cx
                .background_executor()
                .spawn(async move { Arc::new(Self::range_comparison_commit_ids(&repo)) })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this.comparison_order_pending != Some(key) {
                    return;
                }
                this.comparison_order_pending = None;
                if this
                    .active_repo()
                    .is_some_and(|repo| Self::comparison_order_key(repo) == key)
                {
                    this.comparison_order = Some(ComparisonOrderCache {
                        ids: ordered,
                        ..source
                    });
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn comparison_order_cache(
        repo: &RepoState,
        key: u64,
        ids: Arc<Vec<CommitId>>,
    ) -> ComparisonOrderCache {
        ComparisonOrderCache {
            key,
            ids,
            _selection: repo.history_state.multi_selection.commits.clone(),
            _index: repo
                .history_state
                .indexed
                .displayed_index
                .as_ref()
                .or(repo.history_state.indexed.index.as_ref())
                .cloned(),
        }
    }

    fn comparison_count(repo: &RepoState) -> usize {
        if Self::endpoint_range(repo).is_none() {
            repo.history_state.multi_selection.commits.len()
        } else {
            Self::range_comparison_commit_ids(repo).len()
        }
    }

    #[cfg(test)]
    pub(super) fn range_comparison_commits_shared(
        &self,
        repo: &RepoState,
    ) -> std::rc::Rc<[CommitCard]> {
        let ids = Self::range_comparison_commit_ids(repo);
        self.comparison_cards(repo, &ids, 0)
    }

    /// Only the visible IDs and the blocks supplying their metadata invalidate cards.
    pub(super) fn comparison_cards(
        &self,
        repo: &RepoState,
        ids: &[CommitId],
        start: usize,
    ) -> std::rc::Rc<[CommitCard]> {
        use std::hash::{Hash, Hasher};
        let mut hasher = rustc_hash::FxHasher::default();
        repo.id.hash(&mut hasher);
        repo.log_rev.hash(&mut hasher);
        start.hash(&mut hasher);
        ids.hash(&mut hasher);
        let indexed = &repo.history_state.indexed;
        let mut blocks =
            smallvec::SmallVec::<[Arc<gitcomet_core::history_index::HistoryRange>; 4]>::new();
        for id in ids {
            let block = indexed
                .range_index
                .as_ref()
                .and_then(|index| index.position(id.as_ref()))
                .map(|raw| {
                    raw / gitcomet_core::history_index::HISTORY_BLOCK_SIZE
                        * gitcomet_core::history_index::HISTORY_BLOCK_SIZE
                });
            let block = block.and_then(|block| indexed.ranges.get(&block));
            block
                .map(|range| Arc::as_ptr(range) as usize)
                .hash(&mut hasher);
            if let Some(block) = block
                && !blocks.iter().any(|old| Arc::ptr_eq(old, block))
            {
                blocks.push(block.clone());
            }
        }
        let key = hasher.finish();
        if let Some(cache) = &*self.range_comparison_commits_cache.borrow()
            && cache.key == key
        {
            return cache.cards.clone();
        }
        let cards: std::rc::Rc<[CommitCard]> = ids
            .iter()
            .zip(Self::comparison_commits(repo, ids))
            .map(|(id, commit)| {
                gitcomet_core::history_perf::record(
                    gitcomet_core::history_perf::Work::ComparisonCard,
                );
                commit.map_or_else(
                    || CommitCard::unloaded(id),
                    |commit| CommitCard::new(commit.clone()),
                )
            })
            .collect();
        *self.range_comparison_commits_cache.borrow_mut() = Some(ComparisonCardCache {
            key,
            cards: cards.clone(),
            _blocks: blocks.into_vec(),
        });
        cards
    }

    /// One selected/compared-commit preview card: avatar, summary, an author +
    /// relative-time line, and the short SHA. Shared by the multi-selection and
    /// range-comparison lists so both read identically.
    fn commit_card_element(
        &self,
        ix: usize,
        card: &CommitCard,
        now: std::time::SystemTime,
        show_border: bool,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();
        let scaled_px = crate::ui_scale::scaler(self.ui_scale_percent);

        let short_sha = card.short_sha.clone();
        let summary = card.summary.clone();
        let author = card.author.clone();
        let when: SharedString = card
            .unix_secs
            .map(|unix_secs| {
                format!(
                    "{} · {}",
                    author,
                    crate::view::date_time::format_relative_time(unix_secs, now)
                )
            })
            .unwrap_or_default()
            .into();

        div()
            .id(("commit_multi_row", ix))
            .debug_selector(move || format!("commit_multi_row_{ix}"))
            .h(scaled_px(MULTI_COMMIT_ROW_HEIGHT_PX))
            .w_full()
            .flex()
            .items_center()
            .gap(scaled_px(8.0))
            .px(scaled_px(8.0))
            // The last card sits directly above the files section's own top
            // separator, so it omits its bottom border to avoid a double line.
            .when(show_border, |row| {
                row.border_b_1().border_color(theme.colors.stroke.default)
            })
            .when(card.unix_secs.is_some(), |row| {
                row.child(components::author_avatar(theme, ui_scale, author.as_ref()))
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(scaled_px(2.0))
                    .child(
                        div()
                            .text_size(theme.ui_text(14.0))
                            .line_clamp(1)
                            .child(summary),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(12.0))
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(1)
                            .child(when),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(theme.ui_text(12.0))
                    .font_family(crate::view::UI_MONOSPACE_FONT_FAMILY)
                    .text_color(theme.colors.foreground.secondary)
                    .child(short_sha),
            )
            .into_any_element()
    }

    /// Rows for both the comparison view's endpoint cards and the plain
    /// multi-selection list. `range_comparison_commit_ids` already resolves to the
    /// multi-selection when that is what is being compared, so one renderer
    /// serves both and the two views cannot drift apart.
    pub(in crate::view) fn render_multi_commit_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        this.ensure_comparison_order(cx);
        let Some(repo) = this.active_repo() else {
            return Vec::new();
        };
        let key = Self::comparison_order_key(repo);
        let Some(order) = this
            .comparison_order
            .as_ref()
            .filter(|cache| cache.key == key)
        else {
            return Vec::new();
        };
        let ordered = &order.ids;
        let start = range.start.min(ordered.len());
        let end = range.end.min(ordered.len());
        let cards = this.comparison_cards(repo, &ordered[start..end], start);
        let last_ix = ordered.len().saturating_sub(1);
        let now = std::time::SystemTime::now();
        range
            .filter_map(|ix| cards.get(ix - start).map(|card| (ix, card)))
            .map(|(ix, card)| this.commit_card_element(ix, card, now, ix != last_ix))
            .collect()
    }

    /// The scrolling column of commit preview cards, shared by the plain
    /// multi-selection view (where it fills the pane) and the comparison view
    /// (where it is capped and sits above the changed-file list).
    fn commit_cards_list(
        &mut self,
        repo_id: RepoId,
        count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> Stateful<Div> {
        Self::vertical_scroll_frame(
            self.theme,
            ("commit_multi_container", repo_id.0),
            ("commit_multi_scrollbar", repo_id.0),
            &self.commit_multi_scroll,
            uniform_list(
                ("commit_multi_list", repo_id.0),
                count,
                cx.processor(Self::render_multi_commit_rows),
            ),
        )
    }

    pub(super) fn multi_commit_details_view(
        &mut self,
        repo_id: RepoId,
        count: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();

        let header = components::content_header_bar(theme, ui_scale)
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    .line_clamp(1)
                    .child(selected_commits_label(count, Language::current(cx))),
            )
            .child(
                components::Button::new("commit_details_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.foreground.secondary,
                        ui_scale.px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, |this, _e, window, cx| {
                        this.close_review_details(window, cx);
                    })
                    .gitcomet_tooltip(
                        theme,
                        text(cx, "Close commit details", "關閉提交詳情").into(),
                    ),
            );

        // Shown when the selection has no comparison: three or more commits
        // with a gap, commits not in the loaded log, or a selection an
        // external rewrite shrank.
        let no_diff_hint = div()
            .debug_selector(|| "multi_commit_no_merged_diff".to_string())
            .text_size(theme.ui_text(14.0))
            .text_color(theme.colors.foreground.secondary)
            .child(text(cx, "Select two commits to compare them, or consecutive commits to see their combined changes.", "選取兩筆提交來比較，或選取一段連續的提交以查看合併後的變更。"));
        let body = self.commit_cards_list(repo_id, count, cx);

        div()
            .id("commit_details_container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .child(header)
            .child(
                div()
                    .id("commit_details_body_container")
                    .relative()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .p_2()
                    .child(no_diff_hint)
                    .child(body),
            )
            .into_any_element()
    }

    /// The details-pane view shown while two points are being compared: the
    /// selected commit cards, a "viewing diff between" subheader, and the list
    /// of files that differ between them. The diff pane starts empty; clicking a
    /// file loads that file's range diff in the main pane.
    pub(super) fn range_comparison_view(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let ui_scale = self.ui_scale();

        /// What the files section has to say for itself, kept separate from the
        /// count so a failed load can't read as an empty comparison.
        enum RangeFilesState {
            Loading,
            Failed(String),
            Loaded(usize),
        }

        let language = Language::current(cx);
        let (card_count, range, files_state, untracked_notice, toggle_label) = {
            let Some(repo) = self.active_repo() else {
                return div().into_any_element();
            };
            let Some(range) = repo.history_state.range_selection.clone() else {
                return div().into_any_element();
            };
            let card_count = Self::comparison_count(repo);
            let files_state = match &repo.history_state.range_files {
                Loadable::Ready(files) => RangeFilesState::Loaded(files.len()),
                Loadable::Error(e) => RangeFilesState::Failed(e.clone()),
                Loadable::Loading | Loadable::NotLoaded => RangeFilesState::Loading,
            };
            let untracked_notice = comparison_untracked_notice(repo, &range, language);
            let toggle_label = adjacent_comparison_toggle_label(
                &range,
                repo.history_state.multi_selection.commits.len(),
                language,
            );
            (
                card_count,
                range,
                files_state,
                untracked_notice,
                toggle_label,
            )
        };

        let (header_title, subheader) = comparison_titles(&range, card_count, language);
        let revisions = comparison_revision_labels(&range, language, true);
        let full_revisions = comparison_revision_labels(&range, language, false);
        let revision_count = if range.to.is_some() { 2 } else { 1 };

        let header = components::content_header_bar(theme, ui_scale)
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(theme.ui_text(14.0))
                    .font_weight(FontWeight::BOLD)
                    .line_clamp(1)
                    .child(header_title),
            )
            .child(
                components::Button::new("range_comparison_close", "")
                    .start_slot(svg_icon(
                        "icons/generic_close.svg",
                        theme.colors.foreground.secondary,
                        ui_scale.px(12.0),
                    ))
                    .style(components::ButtonStyle::Transparent)
                    .on_click(theme, cx, |this, _e, _w, cx| {
                        if let Some(repo_id) = this.active_repo_id() {
                            this.store.dispatch(Msg::ClearComparison { repo_id });
                        }
                        cx.notify();
                    })
                    .gitcomet_tooltip(theme, text(cx, "Close comparison", "關閉比較").into()),
            );

        // Compared-commit preview cards. A two-point comparison has one or two,
        // but a multi-selection has one per selected commit, so the section
        // grows with the selection only up to half the comparison body and
        // scrolls past that — an even split with the changed-file list below,
        // rather than crowding it out.
        //
        // The cap is relative to the body, so it tracks the pane at whatever
        // height the splitter leaves it. The requested height stays definite
        // though: the card list is a `uniform_list`, which paints nothing when
        // its viewport height is indefinite, so `max_h` does the capping rather
        // than the height itself being content-derived.
        let card_row_height = ui_scale.px(MULTI_COMMIT_ROW_HEIGHT_PX);
        let cards = (card_count > 0).then(|| {
            div()
                .debug_selector(|| "range_comparison_cards".to_string())
                .flex()
                .flex_col()
                .w_full()
                .h(card_row_height * card_count as f32)
                .max_h(relative(COMPARISON_CARDS_MAX_BODY_FRACTION))
                .min_h(card_row_height)
                .child(self.commit_cards_list(repo_id, card_count, cx))
        });

        // No count until there is one: claiming "0 changed" while the diff is
        // still running states a number that is usually about to be wrong. The
        // selector names which of the two the label is, so a test can tell them
        // apart without reading painted text.
        let (files_label, files_label_selector): (SharedString, &'static str) = match &files_state {
            RangeFilesState::Loading | RangeFilesState::Failed(_) => (
                text(cx, "Changed files", "變更的檔案").into(),
                "range_files_label_pending",
            ),
            RangeFilesState::Loaded(count) => {
                let label = match language {
                    Language::English => format!("{count} changed"),
                    Language::TraditionalChinese => format!("{count} 個檔案有變更"),
                };
                (label.into(), "range_files_label_count")
            }
        };
        let (range_row_count, range_counts) = self
            .active_repo()
            .and_then(|repo| {
                let Loadable::Ready(files) = &repo.history_state.range_files else {
                    return None;
                };
                let rev = repo.history_state.range_files_rev;
                let projection = self.cached_range_file_projection(repo_id, rev, files);
                let plan = self.cached_range_file_plan(repo_id, rev, files);
                Some((plan.row_len(), projection.counts))
            })
            .unwrap_or((0, Default::default()));
        let range_controls = self.file_list_controls(
            crate::view::rows::FileListId::RangeFiles,
            repo_id,
            "range_file",
            range_counts.all == 0,
            cx,
        );
        let range_filters_width = self
            .range_filter_bounds_ref
            .borrow()
            .as_ref()
            .map(|b| b.size.width)
            .unwrap_or(Pixels::MAX);
        let range_filters = self.commit_file_filter_tabs(
            crate::view::rows::FileListId::RangeFiles,
            "range_file",
            range_filters_width,
            range_counts,
            cx,
        );

        let files_body: AnyElement = match &files_state {
            RangeFilesState::Loading => div()
                .debug_selector(|| "range_files_loading".to_string())
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child(text(cx, "Loading", "載入中"))
                .into_any_element(),
            // An error must not render as "No files." — that is exactly what a
            // pair of identical commits looks like, so the user would read a
            // failed comparison as a successful, empty one.
            RangeFilesState::Failed(message) => div()
                .debug_selector(|| "range_files_error".to_string())
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.status.danger.foreground)
                .child(SharedString::from(message.clone()))
                .into_any_element(),
            RangeFilesState::Loaded(0) => div()
                .debug_selector(|| "range_files_empty".to_string())
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child(text(cx, "No files.", "沒有檔案。"))
                .into_any_element(),
            RangeFilesState::Loaded(_) => Self::vertical_scroll_frame_content(
                theme,
                ("range_files_container", repo_id.0),
                ("range_files_scrollbar", repo_id.0),
                &self.range_files_scroll,
                Some(
                    self.review_file_focus(crate::view::rows::FileListId::RangeFiles)
                        .clone(),
                ),
                self.changed_file_list(
                    repo_id,
                    crate::view::rows::FileListId::RangeFiles,
                    range_row_count,
                    cx,
                ),
            )
            .into_any_element(),
        };

        div()
            .id("range_comparison_container")
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .child(header)
            .child(
                div()
                    .id("range_comparison_body")
                    .debug_selector(|| "range_comparison_body".to_string())
                    .relative()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .p_2()
                    .child(
                        div()
                            .id("range_comparison_mode")
                            .w_full()
                            .min_w(px(0.0))
                            .text_size(theme.ui_text(14.0))
                            .text_color(theme.colors.foreground.secondary)
                            .line_clamp(2)
                            .gitcomet_tooltip(theme, subheader.clone())
                            .child(subheader),
                    )
                    .when_some(toggle_label.zip(range.to.clone()), |body, (label, to)| {
                        let from = range.from.clone();
                        body.child(
                            div().flex().w_full().min_w(px(0.0)).child(
                                components::Button::new("range_comparison_toggle_mode", label)
                                    .truncate_label()
                                    .on_click(theme, cx, move |this, _, _, _| {
                                        this.store.dispatch(Msg::ToggleAdjacentComparison {
                                            repo_id,
                                            from: from.clone(),
                                            to: to.clone(),
                                        });
                                    })
                                    .gitcomet_tooltip(theme, label.into())
                                    .max_w_full()
                                    .min_w(px(0.0)),
                            ),
                        )
                    })
                    .child(
                        div()
                            .debug_selector(|| "range_comparison_revisions".to_string())
                            .w_full()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .children(
                                revisions
                                    .into_iter()
                                    .zip(full_revisions)
                                    .take(revision_count)
                                    .enumerate()
                                    .map(|(index, (label, full_label))| {
                                        div()
                                            .id(("range_comparison_revision", index))
                                            .w_full()
                                            .min_w(px(0.0))
                                            .text_size(theme.ui_text(11.0))
                                            .text_color(theme.colors.foreground.secondary)
                                            .line_clamp(1)
                                            .gitcomet_tooltip(theme, full_label.into())
                                            .child(label)
                                    }),
                            ),
                    )
                    .when(range.to.is_none(), |body| {
                        let label = if range.options.include_untracked {
                            text(cx, "Hide untracked files", "隱藏未追蹤檔案")
                        } else {
                            text(cx, "Show untracked files", "顯示未追蹤檔案")
                        };
                        let expected = range.clone();
                        body.child(
                            div()
                                .w_full()
                                .min_w(px(0.0))
                                .flex()
                                .flex_col()
                                .gap_1()
                                .when_some(untracked_notice, |scope, notice| {
                                    scope.child(
                                        div()
                                            .id("range_comparison_untracked_notice")
                                            .debug_selector(|| {
                                                "range_comparison_untracked_notice".to_string()
                                            })
                                            .w_full()
                                            .text_size(theme.ui_text(11.0))
                                            .text_color(theme.colors.foreground.secondary)
                                            .line_clamp(2)
                                            .gitcomet_tooltip(theme, notice.clone().into())
                                            .child(notice),
                                    )
                                })
                                .child(
                                    div().flex().w_full().min_w(px(0.0)).child(
                                        components::Button::new(
                                            "range_comparison_toggle_untracked",
                                            label,
                                        )
                                        .truncate_label()
                                        .on_click(theme, cx, move |this, _, _, _| {
                                            this.store.dispatch(
                                                Msg::ToggleWorkingTreeComparisonUntracked {
                                                    repo_id,
                                                    expected: expected.clone(),
                                                },
                                            );
                                        })
                                        .gitcomet_tooltip(theme, label.into())
                                        .max_w_full()
                                        .min_w(px(0.0)),
                                    ),
                                ),
                        )
                    })
                    .children(cards)
                    .child(
                        div()
                            .debug_selector(|| "range_comparison_files".to_string())
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .h_full()
                            // A real floor, not `px(0.0)`: the cards above are
                            // sized from the window, so on a pane shorter than
                            // that this is what stops them from taking the whole
                            // pane and collapsing the list to nothing.
                            .min_h(ui_scale.px(RANGE_FILES_SECTION_MIN_HEIGHT_PX))
                            .border_t_1()
                            .border_color(theme.colors.stroke.subtle)
                            .pt_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_2()
                                    .w_full()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .debug_selector(move || {
                                                files_label_selector.to_string()
                                            })
                                            .text_size(theme.ui_text(14.0))
                                            .text_color(theme.colors.foreground.secondary)
                                            .child(files_label),
                                    )
                                    .child(range_controls),
                            )
                            .child({
                                let bounds = std::rc::Rc::clone(&self.range_filter_bounds_ref);
                                let pane = cx.weak_entity();
                                div()
                                    .relative()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .on_children_prepainted(move |children, _window, app| {
                                        let next = children.first().copied();
                                        let mut measured = bounds.borrow_mut();
                                        if *measured != next {
                                            *measured = next;
                                            // Cached panes must be notified after prepaint.
                                            let pane = pane.clone();
                                            app.defer(move |app| {
                                                let _ = pane.update(app, |_pane, cx| cx.notify());
                                            });
                                        }
                                    })
                                    .child(visible_bounds_probe())
                                    .child(range_filters)
                            })
                            .child(
                                self.review_file_container(
                                    crate::view::rows::FileListId::RangeFiles,
                                    cx,
                                )
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_h(px(0.0))
                                .child(files_body),
                            ),
                    ),
            )
            .into_any_element()
    }
}

/// The comparison view's header and subheader. Only a merged selection is a
/// "merged diff"; a selected pair and an explicit compare name their endpoints.
fn comparison_titles(
    range: &RangeSelection,
    card_count: usize,
    language: Language,
) -> (SharedString, SharedString) {
    let endpoints = || {
        match language {
            Language::English => format!(
                "Two-point comparison: {} → {}",
                range.from_label, range.to_label
            ),
            Language::TraditionalChinese => {
                format!("兩端比較：{} → {}", range.from_label, range.to_label)
            }
        }
        .into()
    };
    match range.source {
        ComparisonSource::MergedSelection => (
            selected_commits_label(card_count, language).into(),
            match language {
                Language::English => format!(
                    "Cumulative changes of {card_count} commits: {} → {}",
                    range.from_label, range.to_label
                ),
                Language::TraditionalChinese => format!(
                    "{card_count} 筆提交的累計變更：{} → {}",
                    range.from_label, range.to_label
                ),
            }
            .into(),
        ),
        ComparisonSource::SelectedPair | ComparisonSource::AdjacentPair => (
            selected_commits_label(card_count, language).into(),
            endpoints(),
        ),
        ComparisonSource::Explicit => (
            match language {
                Language::English => "Comparison",
                Language::TraditionalChinese => "比較",
            }
            .into(),
            endpoints(),
        ),
    }
}

fn adjacent_comparison_toggle_label(
    range: &RangeSelection,
    selected_count: usize,
    language: Language,
) -> Option<&'static str> {
    if selected_count != 2 || range.to.is_none() {
        return None;
    }
    match range.source {
        ComparisonSource::MergedSelection => Some(language.text("Compare endpoints", "比較兩端")),
        ComparisonSource::AdjacentPair => {
            Some(language.text("Show cumulative changes", "查看累計變更"))
        }
        _ => None,
    }
}

/// The effective diff revisions, rather than the selected commit cards.
fn comparison_revision_labels(
    range: &RangeSelection,
    language: Language,
    abbreviated: bool,
) -> [String; 2] {
    let revision = |id: &CommitId| {
        let sha = id.as_ref();
        let shown = if abbreviated {
            sha.get(..12).unwrap_or(sha)
        } else {
            sha
        };
        if gitcomet_core::domain::is_empty_tree_id(sha) {
            let algorithm = if sha.len() == 40 { "SHA-1" } else { "SHA-256" };
            format!(
                "{shown} · {} ({algorithm})",
                language.text("Empty tree", "空樹")
            )
        } else {
            shown.to_string()
        }
    };
    let from = if range.options.base == gitcomet_core::services::ComparisonBase::MergeBase {
        match &range.base {
            Some(base) => format!(
                "{} {} · {}",
                language.text("From", "起點"),
                revision(base),
                language.text("merge base", "共同祖先")
            ),
            None => language
                .text("From merge base (resolving)", "正在解析共同祖先起點")
                .to_string(),
        }
    } else {
        let label = if range.source == ComparisonSource::MergedSelection
            && !gitcomet_core::domain::is_empty_tree_id(range.diff_from().as_ref())
        {
            Some(language.text("oldest selected commit's parent", "最早選取提交的父提交"))
        } else {
            None
        };
        let from = format!(
            "{} {}",
            language.text("From", "起點"),
            revision(range.diff_from())
        );
        match label {
            Some(label) => format!("{from} · {label}"),
            None => from,
        }
    };
    let to = match &range.to {
        Some(id) => format!("{} {}", language.text("To", "終點"), revision(id)),
        None => language.text("To working tree", "終點為工作樹").to_string(),
    };
    [from, to]
}

/// Git's tracked-only working-tree diff omits every untracked path.
fn comparison_untracked_notice(
    repo: &RepoState,
    range: &RangeSelection,
    language: Language,
) -> Option<String> {
    if range.to.is_some() || range.options.include_untracked {
        return None;
    }
    let entries = match &repo.worktree_status {
        Loadable::Ready(entries) => Some(entries.as_slice()),
        Loadable::NotLoaded if repo.worktree_status_rev == 0 => match &repo.status {
            Loadable::Ready(status) => Some(status.unstaged.as_slice()),
            _ => None,
        },
        Loadable::NotLoaded | Loadable::Loading | Loadable::Error(_) => None,
    };
    let Some(entries) = entries else {
        return Some(
            language
                .text(
                    "Tracked files only · untracked file count unavailable",
                    "僅比較已追蹤檔案 · 未追蹤檔案數量尚未可用",
                )
                .to_string(),
        );
    };
    let count = entries
        .iter()
        .filter(|entry| entry.kind == gitcomet_core::domain::FileStatusKind::Untracked)
        .count();
    Some(match language {
        Language::English => format!("Tracked files only · {count} untracked files excluded"),
        Language::TraditionalChinese => format!("僅比較已追蹤檔案 · 已排除 {count} 個未追蹤檔案"),
    })
}

fn selected_commits_label(count: usize, language: Language) -> String {
    match language {
        Language::English => format!("{count} commits selected"),
        Language::TraditionalChinese => format!("已選取 {count} 筆提交"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(source: ComparisonSource) -> RangeSelection {
        RangeSelection {
            source,
            ..RangeSelection::new(
                CommitId("aaaa".into()),
                Some(CommitId("bbbb".into())),
                "aaaa".into(),
                "bbbb".into(),
            )
        }
    }

    #[test]
    fn a_selected_pair_names_its_endpoints_not_a_merged_diff() {
        let (title, subheader) =
            comparison_titles(&range(ComparisonSource::SelectedPair), 2, Language::English);
        assert_eq!(title.as_ref(), "2 commits selected");
        assert_eq!(subheader.as_ref(), "Two-point comparison: aaaa → bbbb");
        assert_eq!(
            comparison_titles(&range(ComparisonSource::AdjacentPair), 2, Language::English),
            (title, subheader),
        );
    }

    #[test]
    fn adjacent_mode_toggle_requires_exactly_two_selected_commit_endpoints() {
        for language in [Language::English, Language::TraditionalChinese] {
            for source in [
                ComparisonSource::MergedSelection,
                ComparisonSource::AdjacentPair,
            ] {
                let mut compared = range(source);
                assert_eq!(
                    adjacent_comparison_toggle_label(&compared, 2, language),
                    Some(if source == ComparisonSource::MergedSelection {
                        language.text("Compare endpoints", "比較兩端")
                    } else {
                        language.text("Show cumulative changes", "查看累計變更")
                    })
                );
                for count in [0, 1, 3] {
                    assert!(adjacent_comparison_toggle_label(&compared, count, language).is_none());
                }
                compared.to = None;
                assert!(adjacent_comparison_toggle_label(&compared, 2, language).is_none());
            }
            for source in [ComparisonSource::Explicit, ComparisonSource::SelectedPair] {
                assert!(adjacent_comparison_toggle_label(&range(source), 2, language).is_none());
            }
        }
    }

    #[test]
    fn a_merged_selection_names_its_commit_count() {
        let (title, subheader) = comparison_titles(
            &range(ComparisonSource::MergedSelection),
            3,
            Language::English,
        );
        assert_eq!(title.as_ref(), "3 commits selected");
        assert_eq!(
            subheader.as_ref(),
            "Cumulative changes of 3 commits: aaaa → bbbb"
        );
    }

    #[test]
    fn an_explicit_comparison_names_its_endpoints() {
        let (title, subheader) =
            comparison_titles(&range(ComparisonSource::Explicit), 2, Language::English);
        assert_eq!(title.as_ref(), "Comparison");
        assert_eq!(subheader.as_ref(), "Two-point comparison: aaaa → bbbb");
    }

    #[test]
    fn localized_comparison_keeps_range_endpoints() {
        let (title, subheader) = comparison_titles(
            &range(ComparisonSource::Explicit),
            2,
            Language::TraditionalChinese,
        );
        assert_eq!(title.as_ref(), "比較");
        assert_eq!(subheader.as_ref(), "兩端比較：aaaa → bbbb");
    }

    #[test]
    fn cumulative_comparison_names_its_parent_revision_and_empty_tree_algorithm() {
        let mut range = range(ComparisonSource::MergedSelection);
        range.from = CommitId("0123456789abcdef0123456789abcdef01234567".into());
        assert_eq!(
            comparison_revision_labels(&range, Language::English, true),
            [
                "From 0123456789ab · oldest selected commit's parent",
                "To bbbb"
            ]
        );
        assert!(
            comparison_revision_labels(&range, Language::TraditionalChinese, false)[0]
                .contains(range.from.as_ref())
        );
        for (id, algorithm) in [
            (gitcomet_core::domain::EMPTY_TREE_ID_SHA1, "SHA-1"),
            (gitcomet_core::domain::EMPTY_TREE_ID_SHA256, "SHA-256"),
        ] {
            range.from = CommitId(id.into());
            let labels = comparison_revision_labels(&range, Language::TraditionalChinese, false);
            assert_eq!(labels[0], format!("起點 {id} · 空樹 ({algorithm})"));
            assert!(!labels[0].contains("父提交"));
        }
    }

    #[test]
    fn comparison_revision_labels_keep_refs_and_show_the_effective_merge_base() {
        let mut range = range(ComparisonSource::Explicit);
        range.from_label = "feature/topic".into();
        range.to_label = "main".into();
        range.options = gitcomet_core::services::ComparisonOptions::merge_base();
        assert_eq!(
            comparison_titles(&range, 2, Language::English).1.as_ref(),
            "Two-point comparison: feature/topic → main"
        );
        assert_eq!(
            comparison_revision_labels(&range, Language::English, false)[0],
            "From merge base (resolving)"
        );
        range.base = Some(CommitId("cccc".into()));
        assert_eq!(
            comparison_revision_labels(&range, Language::English, false),
            ["From cccc · merge base", "To bbbb"]
        );
    }

    #[test]
    fn working_tree_comparison_counts_only_loaded_untracked_paths() {
        use gitcomet_core::domain::{FileStatus, FileStatusKind, RepoSpec, RepoStatus};
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: "/tmp/compare-untracked".into(),
            },
        );
        let entries = Arc::new(vec![
            FileStatus {
                path: "new.txt".into(),
                kind: FileStatusKind::Untracked,
                conflict: None,
            },
            FileStatus {
                path: "tracked.txt".into(),
                kind: FileStatusKind::Modified,
                conflict: None,
            },
        ]);
        let mut range = range(ComparisonSource::Explicit);
        assert_eq!(
            comparison_untracked_notice(&repo, &range, Language::English),
            None
        );
        range.to = None;
        range.to_label = "Working tree".into();
        assert_eq!(
            comparison_revision_labels(&range, Language::English, false)[1],
            "To working tree"
        );
        repo.status = Loadable::Ready(
            RepoStatus {
                staged: Arc::new(Vec::new()),
                unstaged: entries.clone(),
            }
            .into(),
        );
        assert_eq!(
            comparison_untracked_notice(&repo, &range, Language::TraditionalChinese).as_deref(),
            Some("僅比較已追蹤檔案 · 已排除 1 個未追蹤檔案")
        );
        repo.worktree_status = Loadable::Ready(Arc::new(Vec::new()));
        assert_eq!(
            comparison_untracked_notice(&repo, &range, Language::English).as_deref(),
            Some("Tracked files only · 0 untracked files excluded")
        );
        for unavailable in [Loadable::Loading, Loadable::Error("status failed".into())] {
            repo.worktree_status = unavailable;
            assert_eq!(
                comparison_untracked_notice(&repo, &range, Language::English).as_deref(),
                Some("Tracked files only · untracked file count unavailable")
            );
        }
        repo.worktree_status = Loadable::NotLoaded;
        repo.worktree_status_rev = 1;
        assert_eq!(
            comparison_untracked_notice(&repo, &range, Language::English).as_deref(),
            Some("Tracked files only · untracked file count unavailable")
        );
        range.options.include_untracked = true;
        assert_eq!(
            comparison_untracked_notice(&repo, &range, Language::English),
            None
        );
    }
}
