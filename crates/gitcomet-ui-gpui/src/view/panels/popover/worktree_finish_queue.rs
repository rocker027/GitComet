use super::*;
use crate::localization::Language;
use gitcomet_core::worktree_finish::{AgentDispatchAvailability, AgentSessionState};
use gitcomet_state::model::{
    CleanupBlocker, CleanupOutcome, DriftKind, DriftResolution, FinishMode, ItemOutcome, ItemPhase,
    Loadable, RunPhase, SkipReason, SquashContentCheck, WorktreeFinishRunId,
};
use gitcomet_state::msg::{Msg, WorktreeFinishMsg};

fn short_sha(id: &gitcomet_core::domain::CommitId) -> &str {
    id.as_ref().get(0..8).unwrap_or(id.as_ref())
}

fn blocker_text(blocker: &CleanupBlocker, language: Language) -> String {
    match blocker {
        CleanupBlocker::PrimaryWorktree => language
            .text("Primary worktree cannot be removed", "主工作樹無法移除")
            .to_string(),
        CleanupBlocker::BareWorktree => language
            .text("Bare worktree cannot be removed", "Bare 工作樹無法移除")
            .to_string(),
        CleanupBlocker::WorktreeMissing => language
            .text("Worktree folder no longer exists", "工作樹資料夾已不存在")
            .to_string(),
        CleanupBlocker::HeadChangedAfterFinish => language
            .text("New commits appeared after finish", "完成後又有新的提交")
            .to_string(),
        CleanupBlocker::BranchChangedOrDetached => language
            .text("Branch changed or detached", "分支已切換或分離")
            .to_string(),
        CleanupBlocker::DirtyOrUntracked => language
            .text(
                "Uncommitted or untracked files present",
                "有未提交或未追蹤的檔案",
            )
            .to_string(),
        CleanupBlocker::InProgressOperation => language
            .text("Git operation in progress", "有進行中的 Git 操作")
            .to_string(),
        CleanupBlocker::WorktreeLocked => language
            .text("Worktree is locked", "工作樹已鎖定")
            .to_string(),
        CleanupBlocker::HasSubmodules => language
            .text(
                "Contains submodules (Git refuses non-force removal)",
                "含子模組，Git 不會在不強制的情況下移除",
            )
            .to_string(),
        CleanupBlocker::OpenGitCometTerminal => language
            .text(
                "GitComet terminal is still running here",
                "GitComet 終端仍在使用",
            )
            .to_string(),
        CleanupBlocker::ActiveOrcaAgent => language
            .text(
                "An Orca agent is still active here",
                "仍有 agent 在此工作樹工作",
            )
            .to_string(),
        CleanupBlocker::OrcaSessionUnverifiable => language
            .text(
                "Cannot verify whether known agent left this worktree",
                "無法核實已知 agent 是否已離開此工作樹",
            )
            .to_string(),
        CleanupBlocker::IgnoredFilesChangedOrUnverified => language
            .text(
                "Git-ignored files changed since preview disclosure",
                "被忽略檔案較預覽時有異動或無法核對",
            )
            .to_string(),
        CleanupBlocker::ResolutionUnverified => language
            .text(
                "Conflict resolution could not be verified",
                "無法驗明衝突解法或最終狀態",
            )
            .to_string(),
        CleanupBlocker::DisqualifiedByDrift(msg) => msg.clone(),
        CleanupBlocker::GitRefused(msg) => format!("Git refused: {msg}"),
    }
}

pub(super) fn panel(
    this: &mut PopoverHost,
    repo_id: RepoId,
    run_id: WorktreeFinishRunId,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    let theme = this.theme;
    let language = Language::current(cx);
    let scaled_px = super::popover_scaled_px_fn(cx);

    let Some(run) = this
        .state
        .worktree_finish_runs
        .iter()
        .find(|r| r.id == run_id)
        .cloned()
    else {
        return div()
            .flex()
            .flex_col()
            .w(scaled_px(640.0))
            .child(popover_title(
                theme,
                language.text("Worktree finish queue", "工作樹收尾佇列"),
            ))
            .child(super::popover_rule(theme))
            .child(super::popover_detail(
                theme,
                language.text(
                    "This worktree finish run is no longer active.",
                    "此工作樹收尾任務已結束或關閉。",
                ),
            ))
            .child(super::popover_rule(theme))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .flex()
                    .justify_end()
                    .child(dialog_cancel_button(
                        "worktree_finish_queue_close",
                        "worktree_finish_queue_close_hint",
                        theme,
                        cx,
                    )),
            );
    };

    let mode_label = match run.mode {
        FinishMode::MergeInto => format!("Merge into '{}'", run.target.branch),
        FinishMode::RebaseOnto => format!("Rebase onto '{}'", run.target.branch),
    };
    let phase_label = match run.phase {
        RunPhase::Previewing => language.text("Previewing", "預覽中"),
        RunPhase::AwaitingConfirm { .. } => language.text("Awaiting confirm", "待確認"),
        RunPhase::Running { .. } => language.text("Running", "執行中"),
        RunPhase::Paused { .. } => language.text("Paused", "已暫停"),
        RunPhase::CleanupCheck => language.text("Checking cleanup", "重查清理條件中"),
        RunPhase::CleaningUp => language.text("Cleaning up", "清理工作樹中"),
        RunPhase::Finished => language.text("Finished", "已完成"),
        RunPhase::Aborted => language.text("Aborted", "已中止"),
    };

    let mut rows = div()
        .id("worktree_finish_queue_rows")
        .debug_selector(|| "worktree_finish_queue_rows".to_string())
        .px_2()
        .py_1()
        .flex()
        .flex_col()
        .gap_2()
        .max_h(scaled_px(360.0))
        .overflow_y_scroll();

    for (ix, src) in run.sources.iter().enumerate() {
        let status_text = match &src.phase {
            ItemPhase::Queued => {
                if matches!(run.phase, RunPhase::AwaitingConfirm { .. }) && ix >= run.resume_from {
                    let sha_tag = src
                        .locked_head
                        .as_ref()
                        .map(|h| format!(" ({})", short_sha(h)))
                        .unwrap_or_default();
                    match &src.preview {
                        Loadable::Ready(p) => {
                            let conflict_note = if !p.conflict_paths.is_empty() {
                                match language {
                                    Language::English => {
                                        format!(" · {} conflict(s) ⚠", p.conflict_paths.len())
                                    }
                                    Language::TraditionalChinese => {
                                        format!(" · 預判衝突 {} 檔 ⚠", p.conflict_paths.len())
                                    }
                                }
                            } else {
                                String::new()
                            };
                            match language {
                                Language::English => format!(
                                    "Preview ready{sha_tag} · +{}/-{}{conflict_note}",
                                    p.ahead, p.behind
                                ),
                                Language::TraditionalChinese => format!(
                                    "新預覽就緒{sha_tag} · +{}/-{}{conflict_note}",
                                    p.ahead, p.behind
                                ),
                            }
                        }
                        Loadable::Error(err) => format!("⚠ {err}"),
                        _ => language.text("Previewing…", "預覽中…").to_string(),
                    }
                } else {
                    language.text("Queued", "排隊中").to_string()
                }
            }
            ItemPhase::Running(step) => match language {
                Language::English => format!("Running ({step:?})"),
                Language::TraditionalChinese => format!("執行中（{step:?}）"),
            },
            ItemPhase::PausedConflict {
                site,
                stops,
                snapshot,
                ..
            } => match language {
                Language::English => format!(
                    "Conflict ({} file(s), stop #{stops}) at {}",
                    snapshot
                        .unmerged
                        .len()
                        .max(snapshot.conflict_marker_paths.len()),
                    site.display()
                ),
                Language::TraditionalChinese => format!(
                    "衝突：{} 個檔案（第 {stops} 次停頓），於 {}",
                    snapshot
                        .unmerged
                        .len()
                        .max(snapshot.conflict_marker_paths.len()),
                    site.display()
                ),
            },
            ItemPhase::AgentLaunching { agent_id, .. } => match language {
                Language::English => format!("Launching agent {agent_id}…"),
                Language::TraditionalChinese => format!("正在啟動 agent {agent_id}…"),
            },
            ItemPhase::AgentDispatched {
                agent_id,
                session_id,
                agent_state,
                timed_out,
                ..
            } => {
                let state_s = if *timed_out {
                    language.text("Unresponsive (timed out)", "無回應（逾時）")
                } else {
                    match agent_state {
                        Some(AgentSessionState::Working) => language.text("Working", "進行中"),
                        Some(AgentSessionState::Blocked) => language.text("Blocked", "受阻"),
                        Some(AgentSessionState::Waiting) => {
                            language.text("Waiting input", "等待輸入")
                        }
                        Some(AgentSessionState::Done) => {
                            language.text("Ended (rechecking Git)", "已結束（核對 Git 中）")
                        }
                        _ => language.text("Dispatched", "已派工"),
                    }
                };
                match session_id {
                    Some(sid) => format!("Agent {agent_id} ({sid}): {state_s}"),
                    None => format!("Agent {agent_id}: {state_s}"),
                }
            }
            ItemPhase::AwaitingReview { .. } => language
                .text(
                    "Conflicts resolved — review before continuing",
                    "衝突已解，待檢視後繼續",
                )
                .to_string(),
            ItemPhase::AwaitingCredential { .. } => language
                .text(
                    "Waiting for signing passphrase or credentials",
                    "等待簽章密語或認證",
                )
                .to_string(),
            ItemPhase::NeedsDecision { drift, .. } => match drift {
                DriftKind::AdoptCommit {
                    before_head,
                    after_head,
                    ..
                } => match language {
                    Language::English => format!(
                        "Commit detected externally ({} → {}): adopt or rollback?",
                        short_sha(before_head),
                        short_sha(after_head)
                    ),
                    Language::TraditionalChinese => format!(
                        "偵測到外部已提交（{} → {}）：要採納此提交或還原？",
                        short_sha(before_head),
                        short_sha(after_head)
                    ),
                },
                DriftKind::Aborted { .. } => language
                    .text("Operation was aborted externally", "操作已在外部中止")
                    .to_string(),
                DriftKind::SourceMoved {
                    locked_head,
                    current_head,
                    ..
                } => match language {
                    Language::English => format!(
                        "Source moved ({} → {})",
                        short_sha(locked_head),
                        short_sha(current_head)
                    ),
                    Language::TraditionalChinese => format!(
                        "來源分支已移動（{} → {}）",
                        short_sha(locked_head),
                        short_sha(current_head)
                    ),
                },
                DriftKind::TargetMoved {
                    expected_head,
                    current_head,
                    ..
                } => match language {
                    Language::English => format!(
                        "Target moved externally ({} → {})",
                        short_sha(expected_head),
                        short_sha(current_head)
                    ),
                    Language::TraditionalChinese => format!(
                        "目標分支已被外部移動（{} → {}）",
                        short_sha(expected_head),
                        short_sha(current_head)
                    ),
                },
                DriftKind::Unexpected { summary, .. } => summary.clone(),
            },
            ItemPhase::Done(outcome) => match outcome {
                ItemOutcome::FastForwarded { new_head } => match language {
                    Language::English => format!("Fast-forwarded ({})", short_sha(new_head)),
                    Language::TraditionalChinese => format!("已快轉（{}）", short_sha(new_head)),
                },
                ItemOutcome::Merged { commit } => match language {
                    Language::English => format!("Merged ({})", short_sha(commit)),
                    Language::TraditionalChinese => format!("已合併（{}）", short_sha(commit)),
                },
                ItemOutcome::Squashed {
                    commit,
                    content_verified,
                } => {
                    let grade = match content_verified {
                        SquashContentCheck::Verified => language.text("verified", "已驗證"),
                        SquashContentCheck::ReviewedResolution => {
                            language.text("reviewed resolution", "已檢視解法")
                        }
                    };
                    match language {
                        Language::English => format!("Squashed ({}, {grade})", short_sha(commit)),
                        Language::TraditionalChinese => {
                            format!("已 Squash（{}，{grade}）", short_sha(commit))
                        }
                    }
                }
                ItemOutcome::NothingNew => language.text("Nothing new", "沒有新增").to_string(),
                ItemOutcome::Rebased { new_head, commits } => match language {
                    Language::English => format!(
                        "Rebased {commits} commit(s) ({}; target unchanged)",
                        short_sha(new_head)
                    ),
                    Language::TraditionalChinese => {
                        format!("已變基 {commits} 筆（{}；目標未移動）", short_sha(new_head))
                    }
                },
                ItemOutcome::RebasedSquashed { new_head } => match language {
                    Language::English => format!(
                        "Rebased & squashed ({}; target unchanged)",
                        short_sha(new_head)
                    ),
                    Language::TraditionalChinese => {
                        format!("已變基並壓成單筆（{}；目標未移動）", short_sha(new_head))
                    }
                },
            },
            ItemPhase::Skipped(reason) => match reason {
                SkipReason::UserSkipped => language.text("Skipped", "已略過").to_string(),
                SkipReason::AbortedByUser => language.text("Aborted", "已中止").to_string(),
                SkipReason::SourceMovedSkipped => language
                    .text("Skipped (source moved)", "已略過（來源移動）")
                    .to_string(),
            },
            ItemPhase::Failed { message, .. } => format!("✕ {message}"),
        };

        let cleanup_line: Option<String> = match &src.cleanup {
            CleanupOutcome::NotRequested => None,
            CleanupOutcome::Pending => Some(
                language
                    .text("Cleanup: pending batch-end check", "清理：待整批結束重查")
                    .to_string(),
            ),
            CleanupOutcome::Removed => Some(if run.mode == FinishMode::RebaseOnto {
                match language {
                    Language::English => format!(
                        "Folder removed. Local branch '{}' is preserved and NOT merged into '{}'.",
                        src.branch, run.target.branch
                    ),
                    Language::TraditionalChinese => format!(
                        "資料夾已移除，本機分支「{}」保留，尚未併入「{}」。",
                        src.branch, run.target.branch
                    ),
                }
            } else {
                match language {
                    Language::English => format!(
                        "Folder removed. Local branch '{}' is preserved.",
                        src.branch
                    ),
                    Language::TraditionalChinese => {
                        format!("資料夾已移除，本機分支「{}」已保留。", src.branch)
                    }
                }
            }),
            CleanupOutcome::Kept(reason) => Some(match language {
                Language::English => format!("Kept: {}", blocker_text(reason, language)),
                Language::TraditionalChinese => {
                    format!("未清理：{}", blocker_text(reason, language))
                }
            }),
        };

        let mut card = div()
            .debug_selector(move || format!("worktree_finish_queue_item_{ix}"))
            .px_2()
            .py_1()
            .rounded(px(theme.radii.row))
            .border_1()
            .border_color(theme.colors.stroke.subtle)
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(theme.ui_text(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("{}. {}", ix + 1, src.branch)),
                    )
                    .child(
                        div()
                            .text_size(theme.ui_text(11.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(status_text),
                    ),
            );

        if let Some(cleanup_str) = cleanup_line {
            card = card.child(
                div()
                    .text_size(theme.ui_text(11.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(cleanup_str),
            );
        }

        if run.phase == (RunPhase::Paused { current: ix }) {
            card = render_paused_item_actions(
                card, this, &run, src, repo_id, run_id, theme, language, cx,
            );
        }

        rows = rows.child(card);
    }

    let mut notes = div().px_2().py_1().flex().flex_col().gap_1();
    if let Some(note) = &run.temp_worktree_kept_note {
        notes = notes.child(
            div()
                .text_size(theme.ui_text(11.0))
                .text_color(theme.colors.status.warning.foreground)
                .child(note.clone()),
        );
    }
    if run.cleanup_agent_check_unverified {
        notes = notes.child(
            div()
                .text_size(theme.ui_text(11.0))
                .text_color(theme.colors.foreground.secondary)
                .child(language.text(
                    "Orca CLI was unavailable during cleanup; external agent occupancy was not checked.",
                    "清理時 Orca CLI 不可用，未檢查外部 agent 佔用情形（已依 Git 與 GitComet 終端狀態核對）。",
                )),
        );
    }

    // F8: after RefreshPreview, display refreshed preview blockers/warnings and require
    // an explicit second Confirm with the new fingerprint.
    if matches!(
        run.phase,
        RunPhase::Previewing | RunPhase::AwaitingConfirm { .. }
    ) {
        if run.phase == RunPhase::Previewing {
            notes = notes.child(
                div()
                    .text_size(theme.ui_text(12.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(language.text(
                        "Refreshing preview for updated branch state…",
                        "正在重新預覽更新後的分支狀態…",
                    )),
            );
        }
        for blocker in &run.preview_blockers {
            notes = notes.child(
                div()
                    .text_size(theme.ui_text(11.0))
                    .text_color(theme.colors.status.danger.foreground)
                    .child(format!("✕ {blocker}")),
            );
        }
        for warning in &run.preview_warnings {
            notes = notes.child(
                div()
                    .text_size(theme.ui_text(11.0))
                    .text_color(theme.colors.status.warning.foreground)
                    .child(format!("⚠ {warning}")),
            );
        }
    }

    let reconfirm_ready = match run.phase {
        RunPhase::AwaitingConfirm { fingerprint } if run.can_confirm() => Some(fingerprint),
        _ => None,
    };
    let can_abort_refreshed = matches!(
        run.phase,
        RunPhase::Previewing | RunPhase::AwaitingConfirm { .. }
    ) && !run.is_initial_draft();
    let is_finished = !run.is_active();

    div()
        .flex()
        .flex_col()
        .w(scaled_px(640.0))
        .child(popover_title(
            theme,
            format!("{mode_label} · {phase_label}"),
        ))
        .child(super::popover_rule(theme))
        .child(rows)
        .child(notes)
        .child(super::popover_rule(theme))
        .child(
            div()
                .px_2()
                .py_1()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .when(can_abort_refreshed, |d| {
                    d.child(
                        components::Button::new(
                            "worktree_finish_reconfirm_abort",
                            language.text("Abort remaining", "中止剩餘"),
                        )
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store.dispatch(Msg::WorktreeFinish(
                                WorktreeFinishMsg::AbortRemaining {
                                    run_id,
                                    keep_conflict_state: true,
                                },
                            ));
                        })
                        .debug_selector(|| "worktree_finish_reconfirm_abort".to_string()),
                    )
                })
                .when_some(reconfirm_ready, |d, fingerprint| {
                    d.child(
                        components::Button::new(
                            "worktree_finish_reconfirm_go",
                            language.text("Confirm refreshed preview", "確認新預覽並繼續"),
                        )
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            let terminals =
                                super::worktree_finish_setup::running_terminal_paths(this);
                            this.store
                                .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
                                    run_id,
                                    fingerprint,
                                    running_terminal_paths: terminals,
                                }));
                        })
                        .debug_selector(|| "worktree_finish_reconfirm_go".to_string()),
                    )
                })
                .when(is_finished, |d| {
                    d.child(
                        components::Button::new(
                            "worktree_finish_dismiss_run",
                            language.text("Dismiss", "清除紀錄"),
                        )
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, cx| {
                            this.store
                                .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Dismiss {
                                    run_id,
                                }));
                            this.close_popover(cx);
                        })
                        .debug_selector(|| "worktree_finish_dismiss_run".to_string()),
                    )
                })
                .child(dialog_cancel_button(
                    "worktree_finish_queue_close",
                    "worktree_finish_queue_close_hint",
                    theme,
                    cx,
                )),
        )
}

#[allow(clippy::too_many_arguments)]
fn render_paused_item_actions(
    mut card: gpui::Div,
    _this: &PopoverHost,
    run: &gitcomet_state::model::WorktreeFinishRun,
    src: &gitcomet_state::model::FinishSource,
    _repo_id: RepoId,
    run_id: WorktreeFinishRunId,
    theme: AppTheme,
    language: Language,
    cx: &mut gpui::Context<PopoverHost>,
) -> gpui::Div {
    match &src.phase {
        ItemPhase::PausedConflict { site, .. } => {
            if run.mode == FinishMode::MergeInto && src.squash {
                card = card.child(
                    div()
                        .text_size(theme.ui_text(11.0))
                        .text_color(theme.colors.status.warning.foreground)
                        .child(language.text(
                            "This is a Squash merge conflict (no MERGE_HEAD). Resolve conflicts, then use Recheck & Continue or Skip here.",
                            "這是 Squash 合併的衝突（無 MERGE_HEAD）。解完衝突後請在此面板按重新檢查並繼續，或略過此筆。",
                        )),
                );
            }
            let site_open = site.clone();
            let mut action_row = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .child(
                    components::Button::new(
                        "worktree_finish_open_site",
                        language.text("Open in tab", "在分頁開啟"),
                    )
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, cx| {
                        this.store.dispatch(Msg::OpenRepo(site_open.clone()));
                        this.close_popover(cx);
                    })
                    .debug_selector(|| "worktree_finish_open_site".to_string()),
                )
                .child(
                    components::Button::new(
                        "worktree_finish_recheck",
                        language.text("Recheck", "重新檢查"),
                    )
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store
                            .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Recheck { run_id }));
                    })
                    .debug_selector(|| "worktree_finish_recheck".to_string()),
                )
                .child(
                    components::Button::new(
                        "worktree_finish_load_agents",
                        language.text("Orca agents…", "派給 Orca agent…"),
                    )
                    .style(components::ButtonStyle::Outlined)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store
                            .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::LoadAgents {
                                run_id,
                            }));
                    })
                    .debug_selector(|| "worktree_finish_load_agents".to_string()),
                )
                .child(
                    components::Button::new(
                        "worktree_finish_skip",
                        language.text("Skip item", "略過此筆"),
                    )
                    .style(components::ButtonStyle::Danger)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store
                            .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem { run_id }));
                    })
                    .debug_selector(|| "worktree_finish_skip".to_string()),
                )
                .child(
                    components::Button::new(
                        "worktree_finish_abort_remaining",
                        language.text("Abort remaining", "中止剩餘"),
                    )
                    .style(components::ButtonStyle::Danger)
                    .on_click(theme, cx, move |this, _e, _w, _cx| {
                        this.store.dispatch(Msg::WorktreeFinish(
                            WorktreeFinishMsg::AbortRemaining {
                                run_id,
                                keep_conflict_state: true,
                            },
                        ));
                    })
                    .debug_selector(|| "worktree_finish_abort_remaining".to_string()),
                );

            if src.agent_receipt_unknown {
                card = card.child(
                    div()
                        .text_size(theme.ui_text(11.0))
                        .text_color(theme.colors.status.warning.foreground)
                        .child(language.text(
                            "A previous agent launch receipt is uncertain or still active; fresh dispatch is disabled to prevent duplicate agents.",
                            "先前派工收據不明或代理可能仍在執行，已停用重複派工；請先檢查工作區或重新檢查 Git。",
                        )),
                );
            }
            match &run.agent_dispatch.installed {
                Loadable::Ready(agents) if !agents.is_empty() => {
                    for (ix, agent) in agents.iter().enumerate() {
                        let agent_id = agent.id.clone();
                        let label = format!("▶ {}", agent.name);
                        action_row = action_row.child(
                            components::Button::new(
                                format!("worktree_finish_dispatch_agent_{ix}"),
                                label,
                            )
                            .style(components::ButtonStyle::Filled)
                            .disabled(src.agent_receipt_unknown)
                            .on_click(theme, cx, move |this, _e, _w, _cx| {
                                this.store.dispatch(Msg::WorktreeFinish(
                                    WorktreeFinishMsg::DispatchAgent {
                                        run_id,
                                        agent_id: agent_id.clone(),
                                    },
                                ));
                            })
                            .debug_selector(move || format!("worktree_finish_dispatch_agent_{ix}")),
                        );
                    }
                }
                Loadable::Loading => {
                    action_row = action_row.child(
                        div()
                            .text_size(theme.ui_text(11.0))
                            .text_color(theme.colors.foreground.secondary)
                            .child(language.text("Querying Orca…", "查詢 Orca agent 中…")),
                    );
                }
                Loadable::Error(_) | Loadable::Ready(_) => {
                    if matches!(
                        run.agent_dispatch.availability,
                        Some(AgentDispatchAvailability::NotFound)
                    ) {
                        card = card.child(
                            div()
                                .text_size(theme.ui_text(11.0))
                                .text_color(theme.colors.foreground.secondary)
                                .child(language.text(
                                    "Orca CLI not found — resolve conflicts manually in tab.",
                                    "找不到 Orca CLI，請在分頁手動解決衝突。",
                                )),
                        );
                    } else if let Some(err) = &run.agent_dispatch.last_error {
                        card = card.child(
                            div()
                                .text_size(theme.ui_text(11.0))
                                .text_color(theme.colors.status.danger.foreground)
                                .child(err.clone()),
                        );
                    }
                }
                Loadable::NotLoaded => {}
            }

            card = card.child(action_row);
        }
        ItemPhase::AgentLaunching { agent_id, .. } => {
            card = card.child(
                div()
                    .debug_selector(|| "worktree_finish_agent_launching".to_string())
                    .text_size(theme.ui_text(11.0))
                    .text_color(theme.colors.foreground.secondary)
                    .child(match language {
                        Language::English => {
                            format!("Launching {agent_id}… duplicate launch disabled until receipt returns.")
                        }
                        Language::TraditionalChinese => {
                            format!("正在啟動 {agent_id}… 收到啟動收據前禁止重複派工。")
                        }
                    }),
            );
        }
        ItemPhase::AgentDispatched {
            last_assistant_message,
            ..
        } => {
            if let Some(msg) = last_assistant_message {
                card = card.child(
                    div()
                        .text_size(theme.ui_text(11.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(msg.clone()),
                );
            }
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        components::Button::new(
                            "worktree_finish_poll_agent",
                            language.text("Poll agent", "更新 agent 狀態"),
                        )
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store.dispatch(Msg::WorktreeFinish(
                                WorktreeFinishMsg::PollAgent { run_id },
                            ));
                        })
                        .debug_selector(|| "worktree_finish_poll_agent".to_string()),
                    )
                    .child(
                        components::Button::new(
                            "worktree_finish_recheck",
                            language.text("Recheck Git", "重新檢查 Git"),
                        )
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store
                                .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Recheck {
                                    run_id,
                                }));
                        })
                        .debug_selector(|| "worktree_finish_recheck".to_string()),
                    )
                    .child(
                        components::Button::new(
                            "worktree_finish_switch_manual",
                            language.text("Switch to manual", "改為手動"),
                        )
                        .style(components::ButtonStyle::Subtle)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store.dispatch(Msg::WorktreeFinish(
                                WorktreeFinishMsg::SwitchToManual { run_id },
                            ));
                        })
                        .debug_selector(|| "worktree_finish_switch_manual".to_string()),
                    ),
            );
        }
        ItemPhase::AwaitingReview { site, .. } => {
            let site_open = site.clone();
            card =
                card.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(
                            components::Button::new(
                                "worktree_finish_open_site",
                                language.text("Review in tab", "在分頁檢視差異"),
                            )
                            .style(components::ButtonStyle::Outlined)
                            .on_click(theme, cx, move |this, _e, _w, cx| {
                                this.store.dispatch(Msg::OpenRepo(site_open.clone()));
                                this.close_popover(cx);
                            })
                            .debug_selector(|| "worktree_finish_open_site".to_string()),
                        )
                        .child(
                            components::Button::new(
                                "worktree_finish_continue",
                                language.text("Continue", "繼續"),
                            )
                            .style(components::ButtonStyle::Filled)
                            .on_click(theme, cx, move |this, _e, _w, _cx| {
                                this.store.dispatch(Msg::WorktreeFinish(
                                    WorktreeFinishMsg::Continue { run_id },
                                ));
                            })
                            .debug_selector(|| "worktree_finish_continue".to_string()),
                        )
                        .child(
                            components::Button::new(
                                "worktree_finish_skip",
                                language.text("Skip item", "略過此筆"),
                            )
                            .style(components::ButtonStyle::Danger)
                            .on_click(theme, cx, move |this, _e, _w, _cx| {
                                this.store.dispatch(Msg::WorktreeFinish(
                                    WorktreeFinishMsg::SkipItem { run_id },
                                ));
                            })
                            .debug_selector(|| "worktree_finish_skip".to_string()),
                        ),
                );
        }
        ItemPhase::AwaitingCredential { .. } => {
            card = card.child(
                components::Button::new(
                    "worktree_finish_retry_cred",
                    language.text("Retry step", "重試此步驟"),
                )
                .style(components::ButtonStyle::Filled)
                .on_click(theme, cx, move |this, _e, _w, _cx| {
                    this.store
                        .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::RetryCredential {
                            run_id,
                        }));
                })
                .debug_selector(|| "worktree_finish_retry_cred".to_string()),
            );
        }
        ItemPhase::NeedsDecision { drift, .. } => {
            let mut row = div().flex().flex_wrap().items_center().gap_1();
            match drift {
                DriftKind::AdoptCommit { .. } => {
                    row = row
                        .child(
                            components::Button::new(
                                "worktree_finish_adopt_commit",
                                language.text("Adopt commit", "採納這個提交"),
                            )
                            .style(components::ButtonStyle::Filled)
                            .on_click(theme, cx, move |this, _e, _w, _cx| {
                                this.store.dispatch(Msg::WorktreeFinish(
                                    WorktreeFinishMsg::ResolveDrift {
                                        run_id,
                                        choice: DriftResolution::AdoptCommit,
                                    },
                                ));
                            })
                            .debug_selector(|| "worktree_finish_adopt_commit".to_string()),
                        )
                        .child(
                            components::Button::new(
                                "worktree_finish_rollback_commit",
                                language.text("Rollback to previous HEAD", "還原到原 HEAD"),
                            )
                            .style(components::ButtonStyle::Danger)
                            .on_click(theme, cx, move |this, _e, _w, _cx| {
                                this.store.dispatch(Msg::WorktreeFinish(
                                    WorktreeFinishMsg::ResolveDrift {
                                        run_id,
                                        choice: DriftResolution::RollbackToBeforeHead,
                                    },
                                ));
                            })
                            .debug_selector(|| "worktree_finish_rollback_commit".to_string()),
                        );
                }
                DriftKind::Aborted { .. } => {
                    row = row
                        .child(
                            components::Button::new(
                                "worktree_finish_requeue",
                                language.text("Retry item", "重試此筆"),
                            )
                            .style(components::ButtonStyle::Filled)
                            .on_click(theme, cx, move |this, _e, _w, _cx| {
                                this.store.dispatch(Msg::WorktreeFinish(
                                    WorktreeFinishMsg::ResolveDrift {
                                        run_id,
                                        choice: DriftResolution::RequeueItem,
                                    },
                                ));
                            })
                            .debug_selector(|| "worktree_finish_requeue".to_string()),
                        )
                        .child(
                            components::Button::new(
                                "worktree_finish_skip",
                                language.text("Skip item", "略過此筆"),
                            )
                            .style(components::ButtonStyle::Outlined)
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _w, _cx| {
                                    this.store.dispatch(Msg::WorktreeFinish(
                                        WorktreeFinishMsg::ResolveDrift {
                                            run_id,
                                            choice: DriftResolution::SkipItem,
                                        },
                                    ));
                                },
                            ),
                        );
                }
                DriftKind::SourceMoved { .. } => {
                    row = row.child(
                        components::Button::new(
                            "worktree_finish_refresh_source",
                            language.text("Re-preview latest", "重新預覽最新狀態"),
                        )
                        .style(components::ButtonStyle::Filled)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store.dispatch(Msg::WorktreeFinish(
                                WorktreeFinishMsg::ResolveDrift {
                                    run_id,
                                    choice: DriftResolution::RefreshPreview,
                                },
                            ));
                        })
                        .debug_selector(|| "worktree_finish_refresh_source".to_string()),
                    );
                    if run.mode == FinishMode::MergeInto {
                        row = row.child(
                            components::Button::new(
                                "worktree_finish_keep_locked",
                                language.text(
                                    "Keep locked commit (cancels cleanup)",
                                    "維持鎖定提交（取消清理）",
                                ),
                            )
                            .style(components::ButtonStyle::Outlined)
                            .on_click(theme, cx, move |this, _e, _w, _cx| {
                                this.store.dispatch(Msg::WorktreeFinish(
                                    WorktreeFinishMsg::ResolveDrift {
                                        run_id,
                                        choice: DriftResolution::KeepLockedHead,
                                    },
                                ));
                            })
                            .debug_selector(|| "worktree_finish_keep_locked".to_string()),
                        );
                    }
                    row = row.child(
                        components::Button::new(
                            "worktree_finish_skip",
                            language.text("Skip item", "略過此筆"),
                        )
                        .style(components::ButtonStyle::Subtle)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store.dispatch(Msg::WorktreeFinish(
                                WorktreeFinishMsg::ResolveDrift {
                                    run_id,
                                    choice: DriftResolution::SkipItem,
                                },
                            ));
                        }),
                    );
                }
                DriftKind::TargetMoved { .. } => {
                    row = row
                        .child(
                            components::Button::new(
                                "worktree_finish_accept_target",
                                language.text("Re-preview with new target", "以新目標重新預覽"),
                            )
                            .style(components::ButtonStyle::Filled)
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _w, _cx| {
                                    this.store.dispatch(Msg::WorktreeFinish(
                                        WorktreeFinishMsg::ResolveDrift {
                                            run_id,
                                            choice: DriftResolution::RefreshPreview,
                                        },
                                    ));
                                },
                            ),
                        )
                        .child(
                            components::Button::new(
                                "worktree_finish_abort_remaining",
                                language.text("Abort remaining", "中止剩餘"),
                            )
                            .style(components::ButtonStyle::Danger)
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _w, _cx| {
                                    this.store.dispatch(Msg::WorktreeFinish(
                                        WorktreeFinishMsg::ResolveDrift {
                                            run_id,
                                            choice: DriftResolution::AbortRemaining,
                                        },
                                    ));
                                },
                            ),
                        );
                }
                DriftKind::Unexpected { .. } => {
                    row = row
                        .child(
                            components::Button::new(
                                "worktree_finish_recheck",
                                language.text("Recheck", "重新檢查"),
                            )
                            .style(components::ButtonStyle::Outlined)
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _w, _cx| {
                                    this.store.dispatch(Msg::WorktreeFinish(
                                        WorktreeFinishMsg::Recheck { run_id },
                                    ));
                                },
                            ),
                        )
                        .child(
                            components::Button::new(
                                "worktree_finish_skip",
                                language.text("Skip item", "略過此筆"),
                            )
                            .style(components::ButtonStyle::Outlined)
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _w, _cx| {
                                    this.store.dispatch(Msg::WorktreeFinish(
                                        WorktreeFinishMsg::ResolveDrift {
                                            run_id,
                                            choice: DriftResolution::SkipItem,
                                        },
                                    ));
                                },
                            ),
                        )
                        .child(
                            components::Button::new(
                                "worktree_finish_abort_remaining",
                                language.text("Abort remaining", "中止剩餘"),
                            )
                            .style(components::ButtonStyle::Danger)
                            .on_click(
                                theme,
                                cx,
                                move |this, _e, _w, _cx| {
                                    this.store.dispatch(Msg::WorktreeFinish(
                                        WorktreeFinishMsg::ResolveDrift {
                                            run_id,
                                            choice: DriftResolution::AbortRemaining,
                                        },
                                    ));
                                },
                            ),
                        );
                }
            }
            card = card.child(row);
        }
        ItemPhase::Failed { output, .. } => {
            if let Some(out) = output {
                card = card.child(
                    div()
                        .text_size(theme.ui_text(11.0))
                        .text_color(theme.colors.foreground.secondary)
                        .child(out.chars().take(240).collect::<String>()),
                );
            }
            card = card.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        components::Button::new(
                            "worktree_finish_skip",
                            language.text("Skip item", "略過此筆"),
                        )
                        .style(components::ButtonStyle::Outlined)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store
                                .dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem {
                                    run_id,
                                }));
                        }),
                    )
                    .child(
                        components::Button::new(
                            "worktree_finish_abort_remaining",
                            language.text("Abort remaining", "中止剩餘"),
                        )
                        .style(components::ButtonStyle::Danger)
                        .on_click(theme, cx, move |this, _e, _w, _cx| {
                            this.store.dispatch(Msg::WorktreeFinish(
                                WorktreeFinishMsg::AbortRemaining {
                                    run_id,
                                    keep_conflict_state: true,
                                },
                            ));
                        }),
                    ),
            );
        }
        _ => {}
    }
    card
}
