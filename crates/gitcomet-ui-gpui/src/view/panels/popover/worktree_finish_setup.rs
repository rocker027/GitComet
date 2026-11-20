use super::*;
use crate::localization::Language;
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_state::model::{
    FinishMode, FinishSourceRequest, Loadable, RunPhase, TargetHost, WorktreeFinishRun,
};
use gitcomet_state::msg::{Msg, WorktreeFinishMsg};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SetupSourceRow {
    pub(super) path: PathBuf,
    pub(super) branch: Option<String>,
    pub(super) detached: bool,
    pub(super) bare: bool,
    pub(super) is_primary: bool,
    pub(super) selected: bool,
    pub(super) squash: bool,
    pub(super) remove_after: bool,
    pub(super) squash_message: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct SetupDraft {
    pub(super) repo_id: RepoId,
    pub(super) target_branch: String,
    pub(super) mode: FinishMode,
    pub(super) rows: Vec<SetupSourceRow>,
    pub(super) editing_squash_row: Option<usize>,
    pub(super) last_preview_key: Option<u64>,
}

impl SetupDraft {
    pub(super) fn selected_requests(&self) -> Vec<FinishSourceRequest> {
        self.rows
            .iter()
            .filter(|row| row.selected)
            .filter_map(|row| {
                let branch = row.branch.as_ref()?.clone();
                if row.detached || row.bare || branch == self.target_branch {
                    return None;
                }
                Some(FinishSourceRequest {
                    path: row.path.clone(),
                    branch,
                    squash: row.squash,
                    remove_after: row.remove_after && !row.is_primary,
                    squash_message: row.squash_message.clone(),
                })
            })
            .collect()
    }

    pub(super) fn preview_key(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.repo_id.hash(&mut hasher);
        self.target_branch.hash(&mut hasher);
        self.mode.hash(&mut hasher);
        for req in self.selected_requests() {
            req.path.hash(&mut hasher);
            req.branch.hash(&mut hasher);
            req.squash.hash(&mut hasher);
            req.remove_after.hash(&mut hasher);
            req.squash_message.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub(super) fn matches_run(&self, run: &WorktreeFinishRun) -> bool {
        if run.origin_repo_id != self.repo_id
            || run.mode != self.mode
            || run.target.branch != self.target_branch
        {
            return false;
        }
        let selected = self.selected_requests();
        if selected.len() != run.sources.len() {
            return false;
        }
        selected.iter().zip(run.sources.iter()).all(|(req, src)| {
            req.path == src.path
                && req.branch == src.branch
                && req.squash == src.squash
                && req.remove_after == src.remove_after
        })
    }
}

pub(super) fn prepare_setup_draft(
    this: &mut PopoverHost,
    repo_id: RepoId,
    preselected: &[PathBuf],
    target: Option<String>,
    mode: Option<FinishMode>,
    cx: &mut gpui::Context<PopoverHost>,
) {
    this.worktree_finish_target_filter_input
        .update(cx, |input, cx| input.set_text(String::new(), cx));
    let Some(repo) = this.state.repos.iter().find(|r| r.id == repo_id) else {
        this.worktree_finish_setup = None;
        return;
    };

    let preselected_canon: Vec<PathBuf> = preselected
        .iter()
        .cloned()
        .map(canonicalize_or_original)
        .collect();

    let local_branches: Vec<String> = match &repo.branches {
        Loadable::Ready(list) => list.iter().map(|b| b.name.clone()).collect(),
        _ => Vec::new(),
    };
    let current_branch = repo
        .head_branch
        .ready()
        .filter(|b| !b.is_empty() && b.as_str() != "HEAD")
        .cloned();

    let worktrees = match &repo.worktrees {
        Loadable::Ready(list) => list.as_ref().clone(),
        _ => Vec::new(),
    };

    let preselected_branch = worktrees.iter().find_map(|wt| {
        preselected_canon
            .contains(&canonicalize_or_original(wt.path.clone()))
            .then(|| wt.branch.clone())
            .flatten()
    });
    let target_branch = target
        .filter(|t| !t.is_empty())
        .or_else(|| {
            current_branch
                .clone()
                .filter(|b| Some(b) != preselected_branch.as_ref())
        })
        .or_else(|| {
            local_branches
                .iter()
                .find(|b| {
                    matches!(b.as_str(), "main" | "master")
                        && Some(*b) != preselected_branch.as_ref()
                })
                .cloned()
        })
        .or_else(|| {
            local_branches
                .iter()
                .find(|b| Some(*b) != preselected_branch.as_ref())
                .cloned()
        })
        .or(current_branch)
        .unwrap_or_default();

    let mut rows = Vec::new();
    for (idx, wt) in worktrees.iter().enumerate() {
        let is_primary = idx == 0;
        let wt_canon = canonicalize_or_original(wt.path.clone());
        let eligible = !wt.bare
            && !wt.detached
            && wt
                .branch
                .as_deref()
                .is_some_and(|b| !b.is_empty() && b != target_branch);
        let selected = eligible && preselected_canon.contains(&wt_canon);
        rows.push(SetupSourceRow {
            path: wt.path.clone(),
            branch: wt.branch.clone(),
            detached: wt.detached,
            bare: wt.bare,
            is_primary,
            selected,
            // Both per-source options default to false.
            squash: false,
            remove_after: false,
            squash_message: None,
        });
    }

    let mut draft = SetupDraft {
        repo_id,
        target_branch,
        mode: mode.unwrap_or(FinishMode::MergeInto),
        rows,
        editing_squash_row: None,
        last_preview_key: None,
    };

    if !draft.selected_requests().is_empty() && !draft.target_branch.is_empty() {
        let key = draft.preview_key();
        draft.last_preview_key = Some(key);
        let target_branch = draft.target_branch.clone();
        let mode = draft.mode;
        let sources = draft.selected_requests();
        let running_terminal_paths = running_terminal_paths(this);
        this.store
            .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
                repo_id,
                target_branch,
                mode,
                sources,
                running_terminal_paths,
            }));
    }

    this.worktree_finish_setup = Some(draft);
}

pub(super) fn trigger_preview_if_needed(this: &mut PopoverHost) {
    let Some(draft) = this.worktree_finish_setup.as_mut() else {
        return;
    };
    let sources = draft.selected_requests();
    if sources.is_empty() || draft.target_branch.is_empty() {
        draft.last_preview_key = None;
        return;
    }
    let key = draft.preview_key();
    if draft.last_preview_key == Some(key) {
        return;
    }
    draft.last_preview_key = Some(key);
    let repo_id = draft.repo_id;
    let target_branch = draft.target_branch.clone();
    let mode = draft.mode;
    let running_terminal_paths = running_terminal_paths(this);
    this.store
        .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
            repo_id,
            target_branch,
            mode,
            sources,
            running_terminal_paths,
        }));
}

/// Worktrees with an open embedded terminal, from the store's live set that the root view
/// refreshes whenever a terminal opens or closes. Never read the root view here: the dialog
/// also opens while the root view is being updated (command palette), where that read panics.
pub(super) fn running_terminal_paths(this: &PopoverHost) -> Vec<PathBuf> {
    this.store.running_terminal_paths()
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);

    let local_branches: Vec<String> = this
        .state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .and_then(|r| r.branches.ready())
        .map(|list| list.iter().map(|b| b.name.clone()).collect())
        .unwrap_or_default();

    let Some(draft) = this.worktree_finish_setup.clone() else {
        return div()
            .flex()
            .flex_col()
            .w(scaled_px(640.0))
            .child(popover_title(
                theme,
                language.text("Finish worktrees", "收尾工作樹"),
            ))
            .child(super::popover_rule(theme))
            .child(super::popover_detail(
                theme,
                language.text(
                    "No repository is available for worktree finish.",
                    "目前沒有可用的儲存庫可進行工作樹收尾。",
                ),
            ));
    };

    let active_run = this
        .state
        .worktree_finish_runs
        .iter()
        .rev()
        .find(|run| run.origin_repo_id == repo_id)
        .cloned();
    let matching_run = active_run
        .as_ref()
        .filter(|run| draft.matches_run(run))
        .cloned();

    let selected_requests = draft.selected_requests();
    let selected_count = selected_requests.len();
    let squash_count = selected_requests.iter().filter(|r| r.squash).count();
    let remove_count = selected_requests.iter().filter(|r| r.remove_after).count();

    let host_summary = match &matching_run {
        Some(run) => match &run.target.host {
            TargetHost::OpenTab { path, .. } => match language {
                Language::English => format!("Execution worktree: {} (open tab)", path.display()),
                Language::TraditionalChinese => format!("執行地點：{}（已開分頁）", path.display()),
            },
            TargetHost::LinkedWorktree { path } => match language {
                Language::English => {
                    format!("Execution worktree: {} (linked worktree)", path.display())
                }
                Language::TraditionalChinese => {
                    format!("執行地點：{}（連結工作樹）", path.display())
                }
            },
            TargetHost::Temporary { path, .. } => match language {
                Language::English => format!(
                    "Target branch is not checked out; a temporary worktree will be created after confirm at {}",
                    path.display()
                ),
                Language::TraditionalChinese => format!(
                    "目標分支尚未簽出，確認後將於 {} 建立專用暫時工作樹",
                    path.display()
                ),
            },
            TargetHost::NotNeeded => language
                .text(
                    "Execution location: each selected source worktree (target branch will not move)",
                    "執行地點：各個選取的來源工作樹（目標分支不會移動）",
                )
                .to_string(),
        },
        None => match draft.mode {
            FinishMode::MergeInto => language
                .text(
                    "Select source worktrees to preview merge into the target branch.",
                    "請選取來源工作樹以預覽合併至目標分支。",
                )
                .to_string(),
            FinishMode::RebaseOnto => language
                .text(
                    "Select source worktrees to preview rebasing onto the target branch.",
                    "請選取來源工作樹以預覽變基至目標分支。",
                )
                .to_string(),
        },
    };

    let mode_row = super::prompt_footer_row().gap_2().child(
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(language.text("Mode:", "模式：")),
            )
            .child(
                components::Button::new(
                    "worktree_finish_mode_merge",
                    language.text("Merge into", "合併至目標"),
                )
                .style(if draft.mode == FinishMode::MergeInto {
                    components::ButtonStyle::Filled
                } else {
                    components::ButtonStyle::Outlined
                })
                .on_click(theme, cx, |this, _e, _w, cx| {
                    if let Some(draft) = this.worktree_finish_setup.as_mut() {
                        draft.mode = FinishMode::MergeInto;
                    }
                    trigger_preview_if_needed(this);
                    cx.notify();
                })
                .debug_selector(|| "worktree_finish_mode_merge".to_string()),
            )
            .child(
                components::Button::new(
                    "worktree_finish_mode_rebase",
                    language.text("Rebase onto", "變基至目標"),
                )
                .style(if draft.mode == FinishMode::RebaseOnto {
                    components::ButtonStyle::Filled
                } else {
                    components::ButtonStyle::Outlined
                })
                .on_click(theme, cx, |this, _e, _w, cx| {
                    if let Some(draft) = this.worktree_finish_setup.as_mut() {
                        draft.mode = FinishMode::RebaseOnto;
                    }
                    trigger_preview_if_needed(this);
                    cx.notify();
                })
                .debug_selector(|| "worktree_finish_mode_rebase".to_string()),
            ),
    );

    let target_filter = this
        .worktree_finish_target_filter_input
        .read_with(cx, |input, _| input.text().trim().to_ascii_lowercase());
    let filtered_branches: Vec<(usize, &String)> = local_branches
        .iter()
        .enumerate()
        .filter(|(_, name)| {
            target_filter.is_empty() || name.to_ascii_lowercase().contains(&target_filter)
        })
        .collect();

    let mut target_list = div()
        .id("worktree_finish_target_list")
        .debug_selector(|| "worktree_finish_target_list".to_string())
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .max_h(scaled_px(108.0))
        .overflow_y_scroll();

    for (ix, branch_name) in &filtered_branches {
        let ix = *ix;
        let is_active = *branch_name == &draft.target_branch;
        let name_for_click = (*branch_name).clone();
        target_list = target_list.child(
            components::Button::new(
                format!("worktree_finish_target_{ix}"),
                (*branch_name).clone(),
            )
            .style(if is_active {
                components::ButtonStyle::Filled
            } else {
                components::ButtonStyle::Subtle
            })
            .on_click(theme, cx, move |this, _e, _w, cx| {
                if let Some(draft) = this.worktree_finish_setup.as_mut() {
                    draft.target_branch = name_for_click.clone();
                    for row in &mut draft.rows {
                        if row.branch.as_deref() == Some(name_for_click.as_str()) {
                            row.selected = false;
                        }
                    }
                }
                trigger_preview_if_needed(this);
                cx.notify();
            })
            .debug_selector(move || format!("worktree_finish_target_{ix}")),
        );
    }
    if filtered_branches.is_empty() {
        target_list = target_list.child(
            div()
                .text_size(theme.ui_text(11.0))
                .text_color(theme.colors.foreground.secondary)
                .child(language.text("No matching local branches.", "找不到符合的本機分支。")),
        );
    }

    let target_count_label = if local_branches.len() > 12 || !target_filter.is_empty() {
        Some(match language {
            Language::English => format!("({}/{})", filtered_branches.len(), local_branches.len()),
            Language::TraditionalChinese => {
                format!("（{}/{}）", filtered_branches.len(), local_branches.len())
            }
        })
    } else {
        None
    };

    let target_chips = div()
        .px_2()
        .pb_1()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .text_size(theme.ui_text(12.0))
                                .text_color(theme.colors.foreground.secondary)
                                .child(language.text("Target branch:", "目標分支：")),
                        )
                        .when_some(target_count_label, |d, count| {
                            d.child(
                                div()
                                    .text_size(theme.ui_text(11.0))
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(count),
                            )
                        }),
                )
                .child(
                    div()
                        .w(scaled_px(220.0))
                        .debug_selector(|| "worktree_finish_target_filter".to_string())
                        .child(this.worktree_finish_target_filter_input.clone()),
                ),
        )
        .child(target_list);

    let mut rows_container = div()
        .id("worktree_finish_sources_list")
        .debug_selector(|| "worktree_finish_sources_list".to_string())
        .px_2()
        .py_1()
        .flex()
        .flex_col()
        .gap_1()
        .max_h(scaled_px(260.0))
        .overflow_y_scroll();

    if draft.rows.is_empty() {
        rows_container = rows_container.child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.secondary)
                .child(language.text("No worktrees loaded yet.", "尚未載入工作樹清單。")),
        );
    }

    let row_count = draft.rows.len();
    for (ix, row) in draft.rows.iter().enumerate() {
        let ineligible_reason: Option<&'static str> = if row.bare {
            Some(language.text("Bare worktree", "Bare 工作樹"))
        } else if row.detached {
            Some(language.text("Detached HEAD", "分離 HEAD"))
        } else if row.branch.as_deref() == Some(draft.target_branch.as_str()) {
            Some(language.text("Same as target branch", "與目標分支相同"))
        } else if row.branch.is_none() {
            Some(language.text("No local branch", "無本機分支"))
        } else {
            None
        };

        let preview_summary: Option<String> = matching_run.as_ref().and_then(|run| {
            let src = run.sources.iter().find(|s| s.path == row.path)?;
            match &src.preview {
                Loadable::Loading | Loadable::NotLoaded => {
                    Some(language.text("Previewing…", "預覽中…").to_string())
                }
                Loadable::Error(err) => Some(format!("⚠ {err}")),
                Loadable::Ready(p) => {
                    let delta = format!("+{} / -{}", p.ahead, p.behind);
                    let status = if !p.conflict_paths.is_empty() {
                        match language {
                            Language::English => {
                                format!("Conflict ({} file(s)) ⚠", p.conflict_paths.len())
                            }
                            Language::TraditionalChinese => {
                                format!("預判衝突（{} 檔）⚠", p.conflict_paths.len())
                            }
                        }
                    } else if draft.mode == FinishMode::RebaseOnto {
                        if p.target_is_ancestor_of_source && !row.squash {
                            language.text("Nothing new", "沒有新增").to_string()
                        } else if row.squash {
                            language
                                .text("Rebase then Squash (1 commit)", "變基後壓成 1 筆")
                                .to_string()
                        } else {
                            match language {
                                Language::English => format!("Rebase {} commit(s)", p.ahead),
                                Language::TraditionalChinese => {
                                    format!("變基 {} 筆提交", p.ahead)
                                }
                            }
                        }
                    } else if p.source_is_ancestor_of_target
                        || (row.squash && p.merged_tree.is_some() && p.merged_tree == p.target_tree)
                    {
                        language.text("Nothing new", "沒有新增").to_string()
                    } else if row.squash {
                        language
                            .text("1 Squash commit", "1 筆 Squash 提交")
                            .to_string()
                    } else if p.target_is_ancestor_of_source {
                        language.text("Fast-forward", "快轉").to_string()
                    } else {
                        language.text("Merge commit", "合併提交").to_string()
                    };
                    Some(format!("{delta} · {status}"))
                }
            }
        });

        let branch_label = row
            .branch
            .clone()
            .unwrap_or_else(|| language.text("(detached)", "（分離 HEAD）").to_string());
        let short_path = row
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| row.path.display().to_string());

        let can_select = ineligible_reason.is_none();
        let can_remove = can_select && !row.is_primary;

        let mut row_el = div()
            .px_2()
            .py_1()
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(if row.selected {
                theme.colors.accent.foreground
            } else {
                theme.colors.stroke.subtle
            })
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                components::Button::new(
                                    format!("worktree_finish_src_select_{ix}"),
                                    if row.selected { "☑" } else { "☐" },
                                )
                                .style(components::ButtonStyle::Subtle)
                                .disabled(!can_select)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    if let Some(draft) = this.worktree_finish_setup.as_mut()
                                        && let Some(r) = draft.rows.get_mut(ix)
                                    {
                                        r.selected = !r.selected;
                                    }
                                    trigger_preview_if_needed(this);
                                    cx.notify();
                                })
                                .debug_selector(move || format!("worktree_finish_src_select_{ix}")),
                            )
                            .child(
                                components::Button::new(
                                    format!("worktree_finish_src_up_{ix}"),
                                    "▲",
                                )
                                .style(components::ButtonStyle::Subtle)
                                .disabled(ix == 0)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    if let Some(draft) = this.worktree_finish_setup.as_mut()
                                        && ix > 0
                                        && ix < draft.rows.len()
                                    {
                                        draft.rows.swap(ix - 1, ix);
                                    }
                                    trigger_preview_if_needed(this);
                                    cx.notify();
                                })
                                .debug_selector(move || format!("worktree_finish_src_up_{ix}")),
                            )
                            .child(
                                components::Button::new(
                                    format!("worktree_finish_src_down_{ix}"),
                                    "▼",
                                )
                                .style(components::ButtonStyle::Subtle)
                                .disabled(ix + 1 >= row_count)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    if let Some(draft) = this.worktree_finish_setup.as_mut()
                                        && ix + 1 < draft.rows.len()
                                    {
                                        draft.rows.swap(ix, ix + 1);
                                    }
                                    trigger_preview_if_needed(this);
                                    cx.notify();
                                })
                                .debug_selector(move || format!("worktree_finish_src_down_{ix}")),
                            )
                            .child(
                                div()
                                    .text_size(theme.ui_text(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(branch_label),
                            )
                            .child(
                                div()
                                    .text_size(theme.ui_text(11.0))
                                    .text_color(theme.colors.foreground.secondary)
                                    .child(short_path),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                components::Button::new(
                                    format!("worktree_finish_src_squash_{ix}"),
                                    format!(
                                        "{} {}",
                                        if row.squash { "☑" } else { "☐" },
                                        language.text("Squash", "壓成單筆 Squash")
                                    ),
                                )
                                .style(components::ButtonStyle::Subtle)
                                .disabled(!row.selected)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    if let Some(draft) = this.worktree_finish_setup.as_mut()
                                        && let Some(r) = draft.rows.get_mut(ix)
                                    {
                                        r.squash = !r.squash;
                                    }
                                    trigger_preview_if_needed(this);
                                    cx.notify();
                                })
                                .debug_selector(move || format!("worktree_finish_src_squash_{ix}")),
                            )
                            .child(
                                components::Button::new(
                                    format!("worktree_finish_src_remove_{ix}"),
                                    format!(
                                        "{} {}",
                                        if row.remove_after { "☑" } else { "☐" },
                                        language.text("Remove folder", "完成後移除資料夾")
                                    ),
                                )
                                .style(components::ButtonStyle::Subtle)
                                .disabled(!row.selected || !can_remove)
                                .on_click(theme, cx, move |this, _e, _w, cx| {
                                    if let Some(draft) = this.worktree_finish_setup.as_mut()
                                        && let Some(r) = draft.rows.get_mut(ix)
                                        && !r.is_primary
                                    {
                                        r.remove_after = !r.remove_after;
                                    }
                                    trigger_preview_if_needed(this);
                                    cx.notify();
                                })
                                .debug_selector(move || format!("worktree_finish_src_remove_{ix}")),
                            ),
                    ),
            );

        if let Some(reason) = ineligible_reason {
            row_el = row_el.child(
                div()
                    .text_size(theme.ui_text(11.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(reason),
            );
        } else if let Some(summary) = preview_summary {
            row_el = row_el.child(
                div()
                    .text_size(theme.ui_text(11.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(summary),
            );
        }

        if row.selected && row.squash {
            let preview_msg = matching_run
                .as_ref()
                .and_then(|run| run.sources.iter().find(|s| s.path == row.path))
                .and_then(|s| s.squash_message.clone());
            let display_msg = row
                .squash_message
                .clone()
                .or(preview_msg)
                .unwrap_or_else(|| {
                    format!(
                        "Squash merge {} into {}",
                        row.branch.as_deref().unwrap_or(""),
                        draft.target_branch
                    )
                });
            let is_editing = draft.editing_squash_row == Some(ix);
            if is_editing {
                row_el = row_el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .child(this.worktree_finish_squash_input.clone()),
                        )
                        .child(
                            components::Button::new(
                                format!("worktree_finish_squash_msg_save_{ix}"),
                                language.text("Apply", "套用"),
                            )
                            .style(components::ButtonStyle::Outlined)
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _w, cx| {
                                    let text = this
                                        .worktree_finish_squash_input
                                        .read_with(cx, |i, _| i.text().to_string());
                                    if let Some(draft) = this.worktree_finish_setup.as_mut() {
                                        if let Some(r) = draft.rows.get_mut(ix) {
                                            r.squash_message =
                                                (!text.trim().is_empty()).then_some(text);
                                        }
                                        draft.editing_squash_row = None;
                                    }
                                    trigger_preview_if_needed(this);
                                    cx.notify();
                                },
                            ),
                        ),
                );
            } else {
                let msg_for_edit = display_msg.clone();
                row_el = row_el.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_1()
                        .child(
                            div()
                                .text_size(theme.ui_text(11.0))
                                .text_color(theme.colors.foreground.secondary)
                                .child(format!(
                                    "{} {}",
                                    language.text("Squash message:", "Squash 訊息："),
                                    display_msg.lines().next().unwrap_or("")
                                )),
                        )
                        .child(
                            components::Button::new(
                                format!("worktree_finish_squash_msg_edit_{ix}"),
                                language.text("Edit", "編輯"),
                            )
                            .style(components::ButtonStyle::Subtle)
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _w, cx| {
                                    this.worktree_finish_squash_input.update(cx, |input, cx| {
                                        input.set_text(msg_for_edit.clone(), cx);
                                    });
                                    if let Some(draft) = this.worktree_finish_setup.as_mut() {
                                        draft.editing_squash_row = Some(ix);
                                    }
                                    cx.notify();
                                },
                            ),
                        ),
                );
            }
        }

        rows_container = rows_container.child(row_el);
    }

    let mut disclosure_lines: Vec<String> = Vec::new();
    match draft.mode {
        FinishMode::MergeInto => {
            disclosure_lines.push(match language {
                Language::English => format!(
                    "Will sequentially run {selected_count} Merge into '{}'.",
                    draft.target_branch
                ),
                Language::TraditionalChinese => format!(
                    "將依序執行 {selected_count} 筆 Merge into「{}」。",
                    draft.target_branch
                ),
            });
        }
        FinishMode::RebaseOnto => {
            disclosure_lines.push(match language {
                Language::English => format!(
                    "Will rebase {selected_count} branch(es) onto '{}'. Target branch will NOT move and changes are NOT merged into target yet.",
                    draft.target_branch
                ),
                Language::TraditionalChinese => format!(
                    "將變基 {selected_count} 個分支至「{}」，目標分支不會移動，尚未因此併入。",
                    draft.target_branch
                ),
            });
            disclosure_lines.push(
                language
                    .text(
                        "This rewrites each source branch's commit history. Preview via merge-tree checks the final state; intermediate commits may still conflict.",
                        "這會改寫來源分支的提交歷史。merge-tree 僅預判最終狀態，逐筆重放時可能在中途另有衝突。",
                    )
                    .to_string(),
            );
        }
    }
    if remove_count > 0 {
        disclosure_lines.push(match language {
            Language::English => format!(
                "After the batch finishes, {remove_count} verified worktree folder(s) will be removed. Local branches are preserved. Git-ignored files inside removed folders will also be deleted."
            ),
            Language::TraditionalChinese => format!(
                "整批完成後將移除 {remove_count} 個工作樹資料夾，本機分支會保留。資料夾內被 Git 忽略的檔案會一併刪除。"
            ),
        });
    }
    if squash_count > 0 {
        disclosure_lines.push(
            language
                .text(
                    "After Squash, Git will not treat the source branch as merged.",
                    "Squash 後 Git 不會把來源分支視為已合併。",
                )
                .to_string(),
        );
    }

    let mut summary_box = div()
        .debug_selector(|| "worktree_finish_confirm_summary".to_string())
        .px_2()
        .py_1()
        .flex()
        .flex_col()
        .gap_1();

    for line in disclosure_lines {
        summary_box = summary_box.child(
            div()
                .text_size(theme.ui_text(12.0))
                .text_color(theme.colors.foreground.primary)
                .child(line),
        );
    }

    if let Some(run) = &matching_run {
        for blocker in &run.preview_blockers {
            summary_box = summary_box.child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.status.danger.foreground)
                    .child(format!("✕ {blocker}")),
            );
        }
        for warning in &run.preview_warnings {
            summary_box = summary_box.child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.status.warning.foreground)
                    .child(format!("⚠ {warning}")),
            );
        }
    }

    let confirm_ready = matching_run.as_ref().and_then(|run| {
        if !run.can_confirm() {
            return None;
        }
        let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
            return None;
        };
        Some((run.id, fingerprint))
    });

    div()
        .flex()
        .flex_col()
        .w(scaled_px(640.0))
        .child(popover_title(
            theme,
            language.text("Finish worktrees", "收尾工作樹"),
        ))
        .child(super::popover_rule(theme))
        .child(mode_row)
        .child(target_chips)
        .child(super::popover_detail(theme, host_summary))
        .child(super::popover_rule(theme))
        .child(rows_container)
        .child(super::popover_rule(theme))
        .child(summary_box)
        .child(super::popover_rule(theme))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .child(dialog_cancel_button(
                    "worktree_finish_setup_cancel",
                    "worktree_finish_setup_cancel_hint",
                    theme,
                    cx,
                ))
                .child(
                    components::Button::new(
                        "worktree_finish_confirm_go",
                        language.text("Confirm & Start", "確認並開始"),
                    )
                    .style(components::ButtonStyle::Filled)
                    .disabled(confirm_ready.is_none())
                    .on_click(theme, cx, move |this, _e, window, cx| {
                        let Some((run_id, fingerprint)) = confirm_ready else {
                            return;
                        };
                        let terminals = running_terminal_paths(this);
                        this.store
                            .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
                                run_id,
                                fingerprint,
                                running_terminal_paths: terminals,
                            }));
                        this.open_popover_centered(
                            PopoverKind::worktree(
                                repo_id,
                                WorktreePopoverKind::FinishQueue { run_id },
                            ),
                            window,
                            cx,
                        );
                    })
                    .debug_selector(|| "worktree_finish_confirm_go".to_string()),
                ),
        )
}
