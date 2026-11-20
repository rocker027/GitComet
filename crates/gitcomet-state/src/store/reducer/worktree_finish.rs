//! Private reducer owner for the Worktree Finish state machine.

use super::auth;
use super::prepared::ReducerInputs;
use crate::model::{
    AgentDispatchState, AppNotification, AppNotificationKind, AppState, CleanupBlocker,
    CleanupOutcome, DriftKind, DriftResolution, FinishMode, FinishSource, FinishSourceRequest,
    FinishStep, FinishTarget, ItemOutcome, ItemPhase, Loadable, RepoId, RunPhase, SkipReason,
    SquashContentCheck, TargetHost, WorktreeFinishRun, WorktreeFinishRunId,
};
use crate::msg::{
    AbortSafetyBaseline, AgentCheckStatus, Effect, FinishStepAction, ReferenceEffect,
    SourceCleanupCheckResult, SourcePreviewResult, StatusEffect, StepSiteGuard,
    WorktreeFinishEffect, WorktreeFinishInternalMsg, WorktreeFinishMsg,
};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::Error;
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::worktree_finish::{
    AgentDispatchAvailability, AgentLaunch, AgentSession, FinishOperation, FinishSnapshot,
    FinishStepCommandOutput, IgnoredFilesFingerprint, InstalledAgent, TreeId,
    conflict_resolution_prompt, default_merge_branch_message, default_squash_merge_message,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const AGENT_POLL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

pub(super) fn reduce(
    inputs: &ReducerInputs,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    msg: WorktreeFinishMsg,
) -> Vec<Effect> {
    match msg {
        WorktreeFinishMsg::Preview {
            repo_id,
            target_branch,
            mode,
            sources,
            running_terminal_paths,
        } => preview(
            inputs,
            id_alloc,
            state,
            repo_id,
            target_branch,
            mode,
            sources,
            running_terminal_paths,
        ),
        WorktreeFinishMsg::Confirm {
            run_id,
            fingerprint,
            running_terminal_paths,
        } => confirm(state, run_id, fingerprint, running_terminal_paths),
        WorktreeFinishMsg::Recheck { run_id } => recheck(state, run_id),
        WorktreeFinishMsg::Continue { run_id } => continue_item(state, run_id),
        WorktreeFinishMsg::SkipItem { run_id } => skip_item(state, run_id),
        WorktreeFinishMsg::AbortRemaining {
            run_id,
            keep_conflict_state,
        } => abort_remaining(state, run_id, keep_conflict_state),
        WorktreeFinishMsg::LoadAgents { run_id } => load_agents(state, run_id),
        WorktreeFinishMsg::DispatchAgent { run_id, agent_id } => {
            dispatch_agent(inputs, state, run_id, agent_id)
        }
        WorktreeFinishMsg::PollAgent { run_id } => poll_agent(inputs, state, run_id),
        WorktreeFinishMsg::SwitchToManual { run_id } => switch_to_manual(state, run_id),
        WorktreeFinishMsg::ResolveDrift { run_id, choice } => resolve_drift(state, run_id, choice),
        WorktreeFinishMsg::RetryCredential { run_id } => retry_credential(state, run_id),
        WorktreeFinishMsg::Dismiss { run_id } => {
            state
                .worktree_finish_runs
                .retain(|run| run.id != run_id || run.is_active());
            Vec::new()
        }
        WorktreeFinishMsg::UpdateRunningTerminals { run_id, paths } => {
            if let Some(run) = find_run_mut(state, run_id) {
                run.running_terminal_paths = paths;
            }
            Vec::new()
        }
        WorktreeFinishMsg::SyncRunningTerminals { paths } => {
            state.running_terminal_paths = paths.clone();
            for run in &mut state.worktree_finish_runs {
                if run.is_active() {
                    run.running_terminal_paths = paths.clone();
                }
            }
            Vec::new()
        }
    }
}

pub(super) fn reduce_internal(
    inputs: &ReducerInputs,
    state: &mut AppState,
    msg: WorktreeFinishInternalMsg,
) -> Vec<Effect> {
    match msg {
        WorktreeFinishInternalMsg::PreviewLoaded {
            run_id,
            start_index,
            common_dir,
            target_head,
            target_host,
            target_snapshot,
            sources,
            error,
        } => preview_loaded(
            state,
            run_id,
            start_index,
            common_dir,
            target_head,
            target_host,
            target_snapshot,
            sources,
            error,
        ),
        WorktreeFinishInternalMsg::TempWorktreePrepared {
            run_id,
            path,
            result,
        } => temp_worktree_prepared(state, run_id, path, result),
        WorktreeFinishInternalMsg::ItemPreflightLoaded {
            run_id,
            item,
            target_head,
            target_snapshot,
            source_head,
            source_snapshot,
            expected_merged_tree,
            error,
        } => item_preflight_loaded(
            state,
            run_id,
            item,
            target_head,
            target_snapshot,
            source_head,
            source_snapshot,
            expected_merged_tree,
            error,
        ),
        WorktreeFinishInternalMsg::StepFinished {
            run_id,
            item,
            step,
            before,
            after,
            command,
            precheck_violation,
            rebased_range,
            target_is_ancestor,
        } => step_finished(
            state,
            run_id,
            item,
            step,
            before,
            after,
            command,
            precheck_violation,
            rebased_range,
            target_is_ancestor,
        ),
        WorktreeFinishInternalMsg::RecheckLoaded {
            run_id,
            item,
            snapshot,
            target_is_ancestor,
            rebased_range,
        } => recheck_loaded(
            state,
            run_id,
            item,
            snapshot,
            target_is_ancestor,
            rebased_range,
        ),
        WorktreeFinishInternalMsg::AgentsLoaded {
            run_id,
            availability,
            agents,
        } => agents_loaded(state, run_id, availability, agents),
        WorktreeFinishInternalMsg::AgentLaunched {
            run_id,
            item,
            agent_id,
            request_id,
            result,
        } => agent_launched(inputs, state, run_id, item, agent_id, request_id, result),
        WorktreeFinishInternalMsg::AgentPolled {
            run_id,
            item,
            sessions,
        } => agent_polled(inputs, state, run_id, item, sessions),
        WorktreeFinishInternalMsg::CleanupChecked {
            run_id,
            results,
            orca_unavailable,
            live_terminal_paths,
        } => cleanup_checked(
            state,
            run_id,
            results,
            orca_unavailable,
            live_terminal_paths,
        ),
        WorktreeFinishInternalMsg::WorktreeRemoved {
            run_id,
            item,
            result,
        } => worktree_removed(state, run_id, item, result),
        WorktreeFinishInternalMsg::TempWorktreeCleaned {
            run_id,
            removed,
            note,
        } => temp_worktree_cleaned(state, run_id, removed, note),
    }
}

#[allow(clippy::too_many_arguments)]
fn preview(
    inputs: &ReducerInputs,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    repo_id: RepoId,
    target_branch: String,
    mode: FinishMode,
    sources: Vec<FinishSourceRequest>,
    running_terminal_paths: Vec<PathBuf>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let provisional_common_dir = repo
        .common_dir
        .as_ref()
        .filter(|path| !path.as_os_str().is_empty())
        .map(|path| canonicalize_or_original(path.to_path_buf()))
        .unwrap_or_else(|| canonicalize_or_original(repo.spec.workdir.clone()));

    // Reject if another run on the same repository (common_dir) is already active beyond initial draft (T-Q7 / F8 / F10).
    let has_running_for_repo = state.has_active_worktree_finish_run(repo_id);
    if has_running_for_repo {
        state.push_notification(AppNotification {
            time: inputs.now(),
            kind: AppNotificationKind::Warning,
            message: "Another worktree finish run is already in progress for this repository."
                .to_string(),
        });
        return Vec::new();
    }

    // Replace any existing unconfirmed initial draft for this repo (never drop mid-batch refresh runs!).
    state.worktree_finish_runs.retain(|run| {
        !(run.is_initial_draft()
            && (run.origin_repo_id == repo_id
                || (!provisional_common_dir.as_os_str().is_empty()
                    && run.common_dir == provisional_common_dir)))
    });

    let run_id = WorktreeFinishRunId(id_alloc.fetch_add(1, Ordering::Relaxed));
    let finish_sources = sources
        .iter()
        .map(|req| FinishSource {
            path: req.path.clone(),
            branch: req.branch.clone(),
            locked_head: None,
            initial_snapshot: None,
            initial_active_sessions: Vec::new(),
            squash: req.squash,
            remove_after: req.remove_after,
            squash_message: req.squash_message.clone(),
            preview: Loadable::Loading,
            phase: ItemPhase::Queued,
            result_head: None,
            expected_merged_tree: None,
            had_conflict: false,
            conflict_stops: 0,
            user_reviewed_resolution: false,
            dispatched_agent: false,
            agent_receipt_unknown: false,
            pending_retry_operation_id: None,
            conflict_baseline_entries: None,
            pending_squash_range: None,
            cleanup_disqualified: None,
            cleanup: CleanupOutcome::NotRequested,
            cleanup_notes: Vec::new(),
            last_git_output: None,
        })
        .collect();

    let run = WorktreeFinishRun {
        id: run_id,
        origin_repo_id: repo_id,
        common_dir: provisional_common_dir,
        mode,
        target: FinishTarget {
            branch: target_branch.clone(),
            initial_head: None,
            locked_head: None,
            host: match mode {
                FinishMode::MergeInto => TargetHost::NotNeeded,
                FinishMode::RebaseOnto => TargetHost::NotNeeded,
            },
            snapshot: None,
        },
        sources: finish_sources,
        phase: RunPhase::Previewing,
        preview_blockers: Vec::new(),
        preview_warnings: Vec::new(),
        agent_dispatch: AgentDispatchState::default(),
        temp_worktree_kept_note: None,
        cleanup_agent_check_unverified: false,
        running_terminal_paths,
        resume_from: 0,
        next_agent_request_id: 1,
        last_error: None,
    };
    state.worktree_finish_runs.push(run);

    vec![Effect::WorktreeFinish(WorktreeFinishEffect::LoadPreview {
        run_id,
        origin_repo_id: repo_id,
        start_index: 0,
        target_branch,
        mode,
        sources,
    })]
}

#[allow(clippy::too_many_arguments)]
fn preview_loaded(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    start_index: usize,
    common_dir: PathBuf,
    target_head: Option<CommitId>,
    target_host: TargetHost,
    target_snapshot: Option<FinishSnapshot>,
    sources: Vec<SourcePreviewResult>,
    error: Option<String>,
) -> Vec<Effect> {
    let canonical_common = canonicalize_or_original(common_dir);

    // T-Q7: verify no OTHER active non-draft run shares the resolved canonical common_dir.
    let conflict_with_other = state.worktree_finish_runs.iter().any(|other| {
        other.id != run_id
            && other.common_dir == canonical_common
            && other.is_active()
            && !other.is_initial_draft()
    });

    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    if run.phase != RunPhase::Previewing {
        return Vec::new();
    }

    run.common_dir = canonical_common;
    if start_index == 0 {
        run.target.initial_head = target_head.clone();
    }
    run.target.locked_head = target_head;
    // Preserve an already-created temporary target worktree across mid-batch RefreshPreview (F8).
    let keep_existing_temp =
        matches!(
            &run.target.host,
            TargetHost::Temporary { created: true, .. }
        ) && matches!(&target_host, TargetHost::Temporary { created: false, .. });
    if !keep_existing_temp {
        run.target.host = target_host;
    }
    run.target.snapshot = target_snapshot;

    let mut blockers = Vec::new();
    let mut warnings = Vec::new();

    if conflict_with_other {
        blockers
            .push("Another worktree finish run is already active for this repository.".to_string());
    }
    if let Some(err) = error {
        blockers.push(err);
    }
    if run.target.locked_head.is_none() {
        blockers.push(format!(
            "Target branch '{}' could not be resolved to a commit.",
            run.target.branch
        ));
    }
    if run.sources.get(start_index..).is_none_or(<[_]>::is_empty) {
        blockers.push("Select at least one source worktree.".to_string());
    }

    let mut seen_paths = HashSet::new();
    for src in run.sources.iter().take(start_index) {
        seen_paths.insert(canonicalize_or_original(src.path.clone()));
    }
    let mut all_touched_paths: HashSet<PathBuf> = HashSet::new();

    for (offset, preview_res) in sources.into_iter().enumerate() {
        let idx = start_index + offset;
        let Some(src) = run.sources.get_mut(idx) else {
            continue;
        };
        src.locked_head = preview_res.locked_head;
        src.initial_snapshot = preview_res.snapshot;
        src.initial_active_sessions = preview_res.active_sessions;
        if src.squash
            && src
                .squash_message
                .as_deref()
                .is_none_or(|m| m.trim().is_empty())
        {
            src.squash_message = preview_res.default_squash_message;
        }
        match preview_res.preview {
            Ok(p) => {
                for touched in &p.touched_paths {
                    all_touched_paths.insert(touched.clone());
                }
                if p.likely_previously_squashed {
                    warnings.push(format!(
                        "'{}' produces no tree diff against '{}'; it may have already been squash-landed. Consider starting a new branch or resetting to target.",
                        src.branch, run.target.branch
                    ));
                }
                src.preview = Loadable::Ready(p);
            }
            Err(err) => {
                blockers.push(format!("Preview failed for '{}': {err}", src.branch));
                src.preview = Loadable::Error(err.to_string());
            }
        }

        let canonical_src = canonicalize_or_original(src.path.clone());
        if !seen_paths.insert(canonical_src) {
            blockers.push(format!(
                "Duplicate source worktree '{}'.",
                src.path.display()
            ));
        }
        if src.branch == run.target.branch {
            blockers.push(format!(
                "Source branch '{}' cannot be the same as target branch.",
                src.branch
            ));
        }

        if let Some(snap) = &src.initial_snapshot {
            if !snap.worktree_exists {
                blockers.push(format!(
                    "Source worktree '{}' does not exist.",
                    src.path.display()
                ));
            }
            if snap.is_bare {
                blockers.push(format!(
                    "Source worktree '{}' is bare and cannot be finished.",
                    src.path.display()
                ));
            }
            if snap.detached
                || src.locked_head.is_none()
                || snap.branch.as_deref() != Some(src.branch.as_str())
            {
                blockers.push(format!(
                    "Source worktree '{}' is detached or not on '{}'; check out the expected local branch first.",
                    src.path.display(),
                    src.branch
                ));
            }
            match run.mode {
                FinishMode::RebaseOnto => {
                    if !snap.is_clean_tracked()
                        || snap.operation != FinishOperation::None
                        || snap.has_squash_msg
                    {
                        blockers.push(format!(
                            "Rebase requires '{}' to be clean with no in-progress Git operation.",
                            src.branch
                        ));
                    }
                }
                FinishMode::MergeInto => {
                    if !snap.is_completely_clean() || snap.operation != FinishOperation::None {
                        warnings.push(format!(
                            "'{}' has uncommitted or untracked changes; only committed HEAD will be merged and automatic folder removal is disabled.",
                            src.branch
                        ));
                        src.cleanup_disqualified = Some(CleanupBlocker::DirtyOrUntracked);
                    }
                }
            }
            if src.remove_after {
                if snap.is_primary_worktree {
                    src.cleanup_disqualified = Some(CleanupBlocker::PrimaryWorktree);
                    warnings.push(format!(
                        "'{}' is the primary worktree and cannot be removed.",
                        src.path.display()
                    ));
                } else if matches!(
                    &snap.ignored_fingerprint,
                    Some(IgnoredFilesFingerprint::Unverifiable(_))
                ) {
                    warnings.push(format!(
                        "Git-ignored files in '{}' could not be completely fingerprinted; automatic folder removal will keep the worktree.",
                        src.branch
                    ));
                } else if snap.ignored_count > 0 {
                    let sample = snap
                        .ignored_sample
                        .iter()
                        .take(3)
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    warnings.push(format!(
                        "Removing '{}' will also delete {} Git-ignored file(s) ({sample}).",
                        src.branch, snap.ignored_count
                    ));
                }
            }
        } else {
            blockers.push(format!(
                "Could not inspect source worktree '{}'.",
                src.path.display()
            ));
        }
    }

    // Target host checks for MergeInto (including F1 target identity & D7 untracked collision check).
    if run.mode == FinishMode::MergeInto {
        let host_is_existing_checkout = matches!(
            run.target.host,
            TargetHost::OpenTab { .. }
                | TargetHost::LinkedWorktree { .. }
                | TargetHost::Temporary { created: true, .. }
        );
        if let Some(target_snap) = &run.target.snapshot {
            if !target_snap.worktree_exists
                || target_snap.is_bare
                || target_snap.detached
                || target_snap.branch.as_deref() != Some(run.target.branch.as_str())
            {
                blockers.push(format!(
                    "Target worktree is not checked out on '{}' (found {:?}, detached={}).",
                    run.target.branch, target_snap.branch, target_snap.detached
                ));
            }
            if target_snap.operation != FinishOperation::None || target_snap.has_squash_msg {
                blockers.push(format!(
                    "Target worktree for '{}' already has an in-progress Git operation.",
                    run.target.branch
                ));
            }
            if !target_snap.is_clean_tracked() {
                blockers.push(format!(
                    "Target worktree for '{}' has uncommitted changes; clean or commit them first.",
                    run.target.branch
                ));
            }
            if target_snap.untracked_count > 0 {
                let colliding: Vec<String> = target_snap
                    .untracked_paths
                    .iter()
                    .filter(|p| all_touched_paths.contains(*p))
                    .take(5)
                    .map(|p| p.display().to_string())
                    .collect();
                if !colliding.is_empty() {
                    blockers.push(format!(
                        "Target worktree has untracked file(s) that could be overwritten: {}.",
                        colliding.join(", ")
                    ));
                } else {
                    warnings.push(format!(
                        "Target worktree has {} untracked file(s). They do not overlap with source changes.",
                        target_snap.untracked_count
                    ));
                }
            }
        } else if host_is_existing_checkout {
            blockers.push(format!(
                "Could not inspect target worktree for '{}'.",
                run.target.branch
            ));
        }
    }

    run.preview_blockers = blockers;
    run.preview_warnings = warnings;
    let fingerprint = run.compute_fingerprint();
    run.phase = RunPhase::AwaitingConfirm { fingerprint };
    Vec::new()
}

fn confirm(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    fingerprint: u64,
    running_terminal_paths: Vec<PathBuf>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::AwaitingConfirm {
        fingerprint: expected,
    } = run.phase
    else {
        return Vec::new();
    };

    // T-Q8: reject stale fingerprint and trigger fresh preview.
    if fingerprint != expected {
        let origin_repo_id = run.origin_repo_id;
        let target_branch = run.target.branch.clone();
        let mode = run.mode;
        let start_index = run.resume_from;
        let sources: Vec<FinishSourceRequest> = run
            .sources
            .iter()
            .skip(start_index)
            .map(|s| FinishSourceRequest {
                path: s.path.clone(),
                branch: s.branch.clone(),
                squash: s.squash,
                remove_after: s.remove_after,
                squash_message: s.squash_message.clone(),
            })
            .collect();
        run.phase = RunPhase::Previewing;
        run.last_error = Some("Preview expired because inputs changed; re-running preview.".into());
        return vec![Effect::WorktreeFinish(WorktreeFinishEffect::LoadPreview {
            run_id,
            origin_repo_id,
            start_index,
            target_branch,
            mode,
            sources,
        })];
    }

    if !run.can_confirm() {
        return Vec::new();
    }

    if !running_terminal_paths.is_empty() || run.running_terminal_paths.is_empty() {
        run.running_terminal_paths = running_terminal_paths;
    }
    run.last_error = None;
    let start_item = run.resume_from;

    // D1: create temporary worktree only after explicit confirmation.
    if run.mode == FinishMode::MergeInto
        && let TargetHost::Temporary {
            path,
            created: false,
        } = &run.target.host
    {
        let temp_path = path.clone();
        let origin_repo_id = run.origin_repo_id;
        let target_branch = run.target.branch.clone();
        run.phase = RunPhase::Running {
            current: start_item,
        };
        run.sources[start_item].phase = ItemPhase::Running(FinishStep::PrepareTempWorktree);
        return vec![Effect::WorktreeFinish(
            WorktreeFinishEffect::PrepareTempWorktree {
                run_id,
                origin_repo_id,
                target_branch,
                temp_path,
            },
        )];
    }

    start_item_preflight(run, start_item)
}

fn temp_worktree_prepared(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    path: PathBuf,
    result: Result<FinishSnapshot, Error>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let start_item = run.resume_from;
    if run.phase
        != (RunPhase::Running {
            current: start_item,
        })
        || run.sources.get(start_item).map(|s| &s.phase)
            != Some(&ItemPhase::Running(FinishStep::PrepareTempWorktree))
    {
        return Vec::new();
    }
    match result {
        Ok(snapshot) => {
            run.target.host = TargetHost::Temporary {
                path,
                created: true,
            };
            run.target.snapshot = Some(snapshot);
            start_item_preflight(run, start_item)
        }
        Err(err) => {
            run.sources[start_item].phase = ItemPhase::Failed {
                message: format!("Failed to create temporary target worktree: {err}"),
                output: None,
            };
            run.phase = RunPhase::Paused {
                current: start_item,
            };
            Vec::new()
        }
    }
}

fn start_item_preflight(run: &mut WorktreeFinishRun, item: usize) -> Vec<Effect> {
    if item >= run.sources.len() {
        return begin_cleanup_check(run);
    }
    let Some(locked_source_head) = run.sources[item].locked_head.clone() else {
        run.sources[item].phase = ItemPhase::Failed {
            message: "Source commit was not locked during preview.".to_string(),
            output: None,
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    };
    run.phase = RunPhase::Running { current: item };
    run.sources[item].phase = ItemPhase::Running(FinishStep::PreflightItem);
    let compute_expected_tree = run.mode == FinishMode::MergeInto && run.sources[item].squash;
    vec![Effect::WorktreeFinish(
        WorktreeFinishEffect::PreflightItem {
            run_id: run.id,
            origin_repo_id: run.origin_repo_id,
            item,
            expected_common_dir: run.common_dir.clone(),
            mode: run.mode,
            target_branch: run.target.branch.clone(),
            target_host: run.target.host.clone(),
            source_path: run.sources[item].path.clone(),
            source_branch: run.sources[item].branch.clone(),
            locked_source_head,
            compute_expected_tree,
        },
    )]
}

#[allow(clippy::too_many_arguments)]
fn item_preflight_loaded(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    item: usize,
    target_head: Option<CommitId>,
    target_snapshot: Option<FinishSnapshot>,
    source_head: Option<CommitId>,
    source_snapshot: Option<FinishSnapshot>,
    expected_merged_tree: Option<TreeId>,
    error: Option<String>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    if run.phase != (RunPhase::Running { current: item })
        || run.sources.get(item).map(|s| &s.phase)
            != Some(&ItemPhase::Running(FinishStep::PreflightItem))
    {
        return Vec::new();
    }

    if let Some(err) = error {
        run.sources[item].phase = ItemPhase::Failed {
            message: err,
            output: None,
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    }

    // F1: verify target host identity (branch, non-detached, non-bare, matching HEAD) for MergeInto.
    if run.mode == FinishMode::MergeInto {
        let site = execution_site(run, item);
        let Some(tgt_snap) = &target_snapshot else {
            run.sources[item].cleanup_disqualified = Some(CleanupBlocker::DisqualifiedByDrift(
                "Target execution worktree snapshot could not be read.".into(),
            ));
            run.sources[item].phase = ItemPhase::NeedsDecision {
                site,
                drift: DriftKind::Unexpected {
                    summary: "Target execution worktree snapshot is unavailable.".to_string(),
                    snapshot: FinishSnapshot::default(),
                },
            };
            run.phase = RunPhase::Paused { current: item };
            return Vec::new();
        };
        if !tgt_snap.worktree_exists
            || tgt_snap.is_bare
            || tgt_snap.detached
            || tgt_snap.branch.as_deref() != Some(run.target.branch.as_str())
        {
            run.sources[item].cleanup_disqualified = Some(CleanupBlocker::BranchChangedOrDetached);
            run.sources[item].phase = ItemPhase::NeedsDecision {
                site,
                drift: DriftKind::Unexpected {
                    summary: format!(
                        "Target worktree is no longer on '{}' (found {:?}, detached={}).",
                        run.target.branch, tgt_snap.branch, tgt_snap.detached
                    ),
                    snapshot: tgt_snap.clone(),
                },
            };
            run.phase = RunPhase::Paused { current: item };
            return Vec::new();
        }
        if let Some(ref resolved_target) = target_head
            && tgt_snap.head.as_ref() != Some(resolved_target)
        {
            run.sources[item].cleanup_disqualified = Some(CleanupBlocker::BranchChangedOrDetached);
            run.sources[item].phase = ItemPhase::NeedsDecision {
                site,
                drift: DriftKind::Unexpected {
                    summary: "Target worktree HEAD does not match target branch ref.".to_string(),
                    snapshot: tgt_snap.clone(),
                },
            };
            run.phase = RunPhase::Paused { current: item };
            return Vec::new();
        }
    }

    // F1: verify source worktree identity (branch, non-detached, non-bare) before execution.
    if let Some(src_snap) = &source_snapshot {
        if !src_snap.worktree_exists
            || src_snap.is_bare
            || src_snap.detached
            || src_snap.branch.as_deref() != Some(run.sources[item].branch.as_str())
        {
            let site = run.sources[item].path.clone();
            run.sources[item].cleanup_disqualified = Some(CleanupBlocker::BranchChangedOrDetached);
            run.sources[item].phase = ItemPhase::NeedsDecision {
                site,
                drift: DriftKind::Unexpected {
                    summary: format!(
                        "Source worktree is no longer on '{}' (found {:?}, detached={}).",
                        run.sources[item].branch, src_snap.branch, src_snap.detached
                    ),
                    snapshot: src_snap.clone(),
                },
            };
            run.phase = RunPhase::Paused { current: item };
            return Vec::new();
        }
    } else {
        let site = run.sources[item].path.clone();
        run.sources[item].cleanup_disqualified = Some(CleanupBlocker::WorktreeMissing);
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::Unexpected {
                summary: "Source worktree snapshot is unavailable.".to_string(),
                snapshot: FinishSnapshot::default(),
            },
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    }

    // F1: target branch ref must still resolve to a commit.
    if target_head.is_none() {
        let site = execution_site(run, item);
        run.sources[item].cleanup_disqualified = Some(CleanupBlocker::BranchChangedOrDetached);
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::Unexpected {
                summary: format!(
                    "Target branch '{}' could not be resolved before execution.",
                    run.target.branch
                ),
                snapshot: target_snapshot.unwrap_or_default(),
            },
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    }

    // T-Q5: detect external target movement between items.
    if let (Some(expected_target), Some(current_target)) =
        (run.target.locked_head.clone(), target_head.clone())
        && expected_target != current_target
    {
        let site = execution_site(run, item);
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::TargetMoved {
                expected_head: expected_target,
                current_head: current_target,
                snapshot: target_snapshot.unwrap_or_default(),
            },
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    }

    // T-Q4 / T-R8: detect source movement after preview lock.
    if let (Some(locked_src), Some(current_src)) =
        (run.sources[item].locked_head.clone(), source_head.clone())
        && locked_src != current_src
    {
        let site = run.sources[item].path.clone();
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::SourceMoved {
                locked_head: locked_src,
                current_head: current_src,
                snapshot: source_snapshot.unwrap_or_default(),
            },
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    }

    // Verify site cleanliness right before executing.
    let site_snap = match run.mode {
        FinishMode::MergeInto => target_snapshot,
        FinishMode::RebaseOnto => source_snapshot,
    };
    if let Some(snap) = &site_snap
        && (!snap.is_clean_tracked()
            || snap.operation != FinishOperation::None
            || snap.has_squash_msg)
    {
        let site = execution_site(run, item);
        run.sources[item].cleanup_disqualified = Some(CleanupBlocker::DisqualifiedByDrift(
            "Worktree became dirty or started another Git operation before execution.".into(),
        ));
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::Unexpected {
                summary: "Execution worktree is no longer clean.".to_string(),
                snapshot: snap.clone(),
            },
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    }

    run.sources[item].expected_merged_tree = expected_merged_tree;
    dispatch_primary_step(run, item)
}

fn dispatch_primary_step(run: &mut WorktreeFinishRun, item: usize) -> Vec<Effect> {
    let site = execution_site(run, item);
    let source_head = run.sources[item]
        .locked_head
        .clone()
        .expect("locked head verified");
    let target_head = run
        .target
        .locked_head
        .clone()
        .expect("target locked head verified");
    let (step, action, verify_rebase_target) = match (run.mode, run.sources[item].squash) {
        (FinishMode::MergeInto, false) => (
            FinishStep::Merge,
            FinishStepAction::Merge {
                source_head,
                message: default_merge_branch_message(
                    &run.sources[item].branch,
                    &run.target.branch,
                ),
            },
            None,
        ),
        (FinishMode::MergeInto, true) => (
            FinishStep::SquashMerge,
            FinishStepAction::SquashMerge { source_head },
            None,
        ),
        (FinishMode::RebaseOnto, _) => (
            FinishStep::Rebase,
            FinishStepAction::Rebase {
                target_head: target_head.clone(),
            },
            Some(target_head),
        ),
    };
    let guard = build_step_guard(run, item, &step);

    run.phase = RunPhase::Running { current: item };
    run.sources[item].phase = ItemPhase::Running(step.clone());
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
        run_id: run.id,
        origin_repo_id: run.origin_repo_id,
        item,
        step,
        site,
        guard,
        action,
        verify_rebase_target,
        auth: None,
    })]
}

#[allow(clippy::too_many_arguments)]
fn step_finished(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    item: usize,
    step: FinishStep,
    before: FinishSnapshot,
    after: Option<FinishSnapshot>,
    command: Result<FinishStepCommandOutput, Error>,
    precheck_violation: Option<String>,
    rebased_range: Option<(CommitId, usize)>,
    target_is_ancestor: Option<bool>,
) -> Vec<Effect> {
    // Handle signing/auth prompt before mutating item into Failed (T-AUTH).
    if precheck_violation.is_none()
        && let Err(err) = &command
        && let Some(prompt) = auth::auth_prompt_for_worktree_finish(run_id, err)
    {
        if let Some(run) = find_run_mut(state, run_id)
            && run.phase == (RunPhase::Running { current: item })
            && run.sources.get(item).map(|s| &s.phase) == Some(&ItemPhase::Running(step.clone()))
        {
            let site = execution_site(run, item);
            let before_head = before
                .head
                .clone()
                .or_else(|| run.target.locked_head.clone())
                .unwrap_or_else(|| CommitId("".into()));
            run.sources[item].phase = ItemPhase::AwaitingCredential {
                step,
                site,
                before_head,
            };
            run.phase = RunPhase::Paused { current: item };
            auth::install_auth_prompt(state, prompt);
        }
        return Vec::new();
    }

    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    if run.phase != (RunPhase::Running { current: item })
        || run.sources.get(item).map(|s| &s.phase) != Some(&ItemPhase::Running(step.clone()))
    {
        // T-Q6: ignore stale completion for mismatched run/item/step.
        return Vec::new();
    }

    let site = execution_site(run, item);
    if let Some(reason) = precheck_violation {
        // F1: in-gate identity/state check rejected the step BEFORE running any Git write.
        run.sources[item].cleanup_disqualified =
            Some(CleanupBlocker::DisqualifiedByDrift(reason.clone()));
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::Unexpected {
                summary: reason,
                snapshot: after.unwrap_or(before),
            },
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    }

    let before_head = before.head.clone();
    let cmd_output_text = match &command {
        Ok(out) => {
            let combined = out.output.combined();
            (!combined.is_empty()).then_some(combined)
        }
        Err(err) => Some(err.to_string()),
    };
    run.sources[item].last_git_output = cmd_output_text.clone();

    let Some(after) = after else {
        let message = command
            .err()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Could not read post-step Git snapshot.".to_string());
        run.sources[item].phase = ItemPhase::Failed {
            message,
            output: cmd_output_text,
        };
        run.phase = RunPhase::Paused { current: item };
        return Vec::new();
    };

    let succeeded = command.as_ref().is_ok_and(|out| out.succeeded);
    let target_branch_matches =
        !after.detached && after.branch.as_deref() == Some(run.target.branch.as_str());
    let source_branch_matches =
        !after.detached && after.branch.as_deref() == Some(run.sources[item].branch.as_str());

    match step {
        FinishStep::Merge => {
            let source_locked = run.sources[item].locked_head.clone();
            if !target_branch_matches {
                run.sources[item].cleanup_disqualified =
                    Some(CleanupBlocker::BranchChangedOrDetached);
                return fail_item(
                    run,
                    item,
                    "Merge ended on an unexpected branch or detached HEAD.",
                    cmd_output_text,
                );
            }
            if after.operation == FinishOperation::Merge {
                run.sources[item].conflict_baseline_entries = after.dirty_entries.clone();
                if !after.unmerged.is_empty() || !after.conflict_marker_paths.is_empty() {
                    run.sources[item].had_conflict = true;
                    run.sources[item].phase = ItemPhase::PausedConflict {
                        site,
                        stops: 1,
                        before_head: before_head.unwrap_or_else(|| CommitId("".into())),
                        snapshot: after,
                    };
                } else {
                    // T-M6: rerere autoUpdate staged resolution -> never auto-commit.
                    run.sources[item].had_conflict = true;
                    run.sources[item].phase = ItemPhase::AwaitingReview {
                        site,
                        stops: 1,
                        before_head: before_head.unwrap_or_else(|| CommitId("".into())),
                        snapshot: after,
                    };
                }
                run.phase = RunPhase::Paused { current: item };
                return Vec::new();
            }
            if after.operation == FinishOperation::None && succeeded {
                if after.head == before_head {
                    run.sources[item].result_head = source_locked;
                    run.sources[item].phase = ItemPhase::Done(ItemOutcome::NothingNew);
                    return advance_after_item(state, run_id, item);
                }
                if let Some(new_head) = after.head.clone() {
                    let outcome = if Some(&new_head) == source_locked.as_ref() {
                        ItemOutcome::FastForwarded {
                            new_head: new_head.clone(),
                        }
                    } else if after.head_parents.len() >= 2 {
                        ItemOutcome::Merged {
                            commit: new_head.clone(),
                        }
                    } else {
                        run.sources[item].phase = ItemPhase::Failed {
                            message: "Merge ended with unexpected commit shape.".to_string(),
                            output: cmd_output_text,
                        };
                        run.phase = RunPhase::Paused { current: item };
                        return Vec::new();
                    };
                    run.target.locked_head = Some(new_head);
                    run.sources[item].result_head = source_locked;
                    run.sources[item].phase = ItemPhase::Done(outcome);
                    return advance_after_item(state, run_id, item);
                }
            }
            fail_item(run, item, "Merge step failed.", cmd_output_text)
        }
        FinishStep::SquashMerge => {
            let source_locked = run.sources[item].locked_head.clone();
            if !target_branch_matches {
                run.sources[item].cleanup_disqualified =
                    Some(CleanupBlocker::BranchChangedOrDetached);
                return fail_item(
                    run,
                    item,
                    "Squash merge ended on an unexpected branch or detached HEAD.",
                    cmd_output_text,
                );
            }
            if !after.unmerged.is_empty() || !after.conflict_marker_paths.is_empty() {
                run.sources[item].had_conflict = true;
                run.sources[item].conflict_baseline_entries = after.dirty_entries.clone();
                run.sources[item].phase = ItemPhase::PausedConflict {
                    site,
                    stops: 1,
                    before_head: before_head.unwrap_or_else(|| CommitId("".into())),
                    snapshot: after,
                };
                run.phase = RunPhase::Paused { current: item };
                return Vec::new();
            }
            // F3: NothingNew REQUIRES command success (exit 0) AND verified matching tree content!
            if succeeded
                && after.operation == FinishOperation::None
                && after.head == before_head
                && after.unmerged.is_empty()
                && after.staged_count == 0
                && after.unstaged_count == 0
            {
                let content_verified =
                    if let Some(expected) = &run.sources[item].expected_merged_tree {
                        after.head_tree.as_ref() == Some(expected)
                    } else {
                        matches!(
                            &run.sources[item].preview,
                            Loadable::Ready(p)
                                if p.source_is_ancestor_of_target
                                    || (after.head == run.target.initial_head
                                        && p.merged_tree.is_some()
                                        && p.merged_tree == p.target_tree)
                        )
                    };
                if content_verified {
                    // T-S4: verified no tree diff -> NothingNew without empty commit.
                    run.sources[item].result_head = source_locked;
                    run.sources[item].phase = ItemPhase::Done(ItemOutcome::NothingNew);
                    return advance_after_item(state, run_id, item);
                }
                return fail_item(
                    run,
                    item,
                    "Squash merge left no staged changes and could not verify target already contains source content.",
                    cmd_output_text,
                );
            }
            if after.head == before_head && after.unmerged.is_empty() && after.staged_count > 0 {
                if succeeded {
                    // Clean squash merge -> commit via GitComet.
                    let message = resolved_squash_message(run, item);
                    let guard = build_step_guard(run, item, &FinishStep::SquashCommit);
                    run.sources[item].phase = ItemPhase::Running(FinishStep::SquashCommit);
                    return vec![Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
                        run_id,
                        origin_repo_id: run.origin_repo_id,
                        item,
                        step: FinishStep::SquashCommit,
                        site,
                        guard,
                        action: FinishStepAction::SquashCommit { message },
                        verify_rebase_target: None,
                        auth: None,
                    })];
                } else if after.has_squash_msg {
                    // T-S6: non-zero exit with empty unmerged + SQUASH_MSG = rerere.autoUpdate staged resolution.
                    run.sources[item].had_conflict = true;
                    run.sources[item].conflict_baseline_entries = after.dirty_entries.clone();
                    run.sources[item].phase = ItemPhase::AwaitingReview {
                        site,
                        stops: 1,
                        before_head: before_head.unwrap_or_else(|| CommitId("".into())),
                        snapshot: after,
                    };
                    run.phase = RunPhase::Paused { current: item };
                    return Vec::new();
                }
            }
            fail_item(run, item, "Squash merge step failed.", cmd_output_text)
        }
        FinishStep::SquashCommit => verify_and_complete_squash_commit(
            state,
            run_id,
            item,
            before_head,
            after,
            succeeded,
            cmd_output_text,
        ),
        FinishStep::ConcludeMerge => {
            let source_locked = run.sources[item].locked_head.clone();
            let valid = succeeded
                && target_branch_matches
                && after.operation == FinishOperation::None
                && after.unmerged.is_empty()
                && after.head != before_head
                && after.head_parents.len() >= 2;
            if valid && let Some(commit) = after.head.clone() {
                run.target.locked_head = Some(commit.clone());
                run.sources[item].result_head = source_locked;
                run.sources[item].user_reviewed_resolution = true;
                run.sources[item].conflict_baseline_entries = None;
                run.sources[item].phase = ItemPhase::Done(ItemOutcome::Merged { commit });
                return advance_after_item(state, run_id, item);
            }
            fail_item(
                run,
                item,
                "Concluding merge commit failed.",
                cmd_output_text,
            )
        }
        FinishStep::Rebase | FinishStep::RebaseContinue => {
            if after.operation == FinishOperation::Rebase {
                run.sources[item].had_conflict = true;
                run.sources[item].conflict_baseline_entries = after.dirty_entries.clone();
                run.sources[item].conflict_stops =
                    run.sources[item].conflict_stops.saturating_add(1);
                let stops = run.sources[item].conflict_stops;
                run.sources[item].phase = ItemPhase::PausedConflict {
                    site,
                    stops,
                    before_head: run.sources[item]
                        .locked_head
                        .clone()
                        .or(before_head)
                        .unwrap_or_else(|| CommitId("".into())),
                    snapshot: after,
                };
                run.phase = RunPhase::Paused { current: item };
                return Vec::new();
            }
            if after.operation == FinishOperation::None
                && succeeded
                && source_branch_matches
                && target_is_ancestor == Some(true)
                && let Some(new_head) = after.head.clone()
            {
                let original_locked = run.sources[item].locked_head.clone();
                let count = rebased_range.as_ref().map(|(_, c)| *c).unwrap_or(0);
                run.sources[item].conflict_baseline_entries = None;
                if run.sources[item].squash
                    && let Some((oldest, c)) = rebased_range
                    && c > 1
                {
                    let message = resolved_squash_message(run, item);
                    run.sources[item].pending_squash_range =
                        Some((oldest.clone(), new_head.clone()));
                    let guard = build_step_guard(run, item, &FinishStep::SquashRange);
                    run.sources[item].phase = ItemPhase::Running(FinishStep::SquashRange);
                    return vec![Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
                        run_id,
                        origin_repo_id: run.origin_repo_id,
                        item,
                        step: FinishStep::SquashRange,
                        site,
                        guard,
                        action: FinishStepAction::SquashRange {
                            oldest,
                            expected_head: new_head,
                            message,
                        },
                        verify_rebase_target: None,
                        auth: None,
                    })];
                }
                let already_on_target = Some(&new_head) == original_locked.as_ref()
                    && (count == 0
                        || (!run.sources[item].had_conflict
                            && matches!(
                                &run.sources[item].preview,
                                Loadable::Ready(p) if p.target_is_ancestor_of_source
                            )));
                let outcome = if already_on_target {
                    ItemOutcome::NothingNew
                } else {
                    ItemOutcome::Rebased {
                        new_head: new_head.clone(),
                        commits: count,
                    }
                };
                run.sources[item].result_head = Some(new_head);
                if run.sources[item].had_conflict {
                    run.sources[item].user_reviewed_resolution = true;
                }
                run.sources[item].phase = ItemPhase::Done(outcome);
                return advance_after_item(state, run_id, item);
            }
            if !source_branch_matches && after.operation == FinishOperation::None {
                run.sources[item].cleanup_disqualified =
                    Some(CleanupBlocker::BranchChangedOrDetached);
                return fail_item(
                    run,
                    item,
                    "Rebase ended on an unexpected branch or detached HEAD.",
                    cmd_output_text,
                );
            }
            fail_item(run, item, "Rebase step failed.", cmd_output_text)
        }
        FinishStep::SquashRange => {
            let target_head = run.target.locked_head.clone();
            let valid = succeeded
                && source_branch_matches
                && after.operation == FinishOperation::None
                && after.head.is_some()
                && target_head
                    .as_ref()
                    .is_some_and(|t| after.head_parents == vec![t.clone()]);
            if valid && let Some(new_head) = after.head.clone() {
                run.sources[item].pending_squash_range = None;
                run.sources[item].result_head = Some(new_head.clone());
                if run.sources[item].had_conflict {
                    run.sources[item].user_reviewed_resolution = true;
                }
                run.sources[item].phase =
                    ItemPhase::Done(ItemOutcome::RebasedSquashed { new_head });
                return advance_after_item(state, run_id, item);
            }
            fail_item(
                run,
                item,
                "Squashing rebased commits failed.",
                cmd_output_text,
            )
        }
        FinishStep::AbortItem => {
            let head_ok = run.mode != FinishMode::MergeInto || after.head == before_head;
            let clean_abort = succeeded
                && head_ok
                && after.operation == FinishOperation::None
                && after.unmerged.is_empty()
                && !after.has_squash_msg;
            if clean_abort {
                run.sources[item].conflict_baseline_entries = None;
                let is_aborting_run = run
                    .sources
                    .iter()
                    .skip(item + 1)
                    .all(|s| matches!(s.phase, ItemPhase::Skipped(SkipReason::AbortedByUser)));
                let reason = if is_aborting_run && item + 1 < run.sources.len() {
                    SkipReason::AbortedByUser
                } else {
                    SkipReason::UserSkipped
                };
                run.sources[item].phase = ItemPhase::Skipped(reason);
                if reason == SkipReason::AbortedByUser {
                    return begin_cleanup_check(run);
                }
                return advance_after_item(state, run_id, item);
            }
            // D7 / F4: if abort could not safely restore (or post-conflict user data was detected),
            // preserve the worktree & index untouched and pause.
            run.sources[item].cleanup_disqualified = Some(CleanupBlocker::DisqualifiedByDrift(
                "Abort could not cleanly restore the worktree without risking user changes.".into(),
            ));
            let msg = cmd_output_text.clone().unwrap_or_else(|| {
                "Could not safely abort conflicted state; worktree preserved for manual recovery."
                    .to_string()
            });
            fail_item(run, item, &msg, cmd_output_text)
        }
        FinishStep::RollbackToHead { head } => {
            if succeeded
                && after.operation == FinishOperation::None
                && after.head.as_ref() == Some(&head)
            {
                run.sources[item].phase = ItemPhase::Queued;
                run.sources[item].had_conflict = false;
                run.sources[item].user_reviewed_resolution = false;
                run.sources[item].conflict_baseline_entries = None;
                run.phase = RunPhase::Paused { current: item };
                return Vec::new();
            }
            fail_item(
                run,
                item,
                "Rollback to previous HEAD failed.",
                cmd_output_text,
            )
        }
        FinishStep::PrepareTempWorktree | FinishStep::PreflightItem => Vec::new(),
    }
}

fn verify_and_complete_squash_commit(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    item: usize,
    before_head: Option<CommitId>,
    after: FinishSnapshot,
    succeeded: bool,
    cmd_output_text: Option<String>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let branch_ok = !after.detached && after.branch.as_deref() == Some(run.target.branch.as_str());
    let structural_ok = succeeded
        && branch_ok
        && after.operation == FinishOperation::None
        && after.unmerged.is_empty()
        && after.staged_count == 0
        && !after.has_squash_msg
        && before_head.as_ref().is_some_and(|bh| {
            after.head.as_ref() != Some(bh) && after.head_parents == vec![bh.clone()]
        });
    if !structural_ok {
        return fail_item(
            run,
            item,
            "Squash commit failed structural verification (expected single-parent commit advancing target HEAD).",
            cmd_output_text,
        );
    }

    let content_verified = if !run.sources[item].had_conflict {
        // §3.4 & T-S5: clean squash MUST match pre-step merge-tree tree!
        let expected_tree = run.sources[item].expected_merged_tree.clone();
        match (expected_tree.as_ref(), after.head_tree.as_ref()) {
            (Some(expected), Some(actual)) if expected == actual => SquashContentCheck::Verified,
            (Some(expected), actual) => {
                run.sources[item].cleanup_disqualified = Some(CleanupBlocker::ResolutionUnverified);
                let msg = format!(
                    "Squash commit tree ({actual:?}) did not match pre-step merge-tree ({expected}); index may have been modified externally."
                );
                return fail_item(run, item, &msg, cmd_output_text);
            }
            (None, _) => {
                run.sources[item].cleanup_disqualified = Some(CleanupBlocker::ResolutionUnverified);
                return fail_item(
                    run,
                    item,
                    "Clean squash commit could not be verified because pre-step merge-tree expectation is unavailable.",
                    cmd_output_text,
                );
            }
        }
    } else {
        run.sources[item].user_reviewed_resolution = true;
        SquashContentCheck::ReviewedResolution
    };

    let commit = after.head.clone().expect("verified head");
    let source_locked = run.sources[item].locked_head.clone();
    run.target.locked_head = Some(commit.clone());
    run.sources[item].result_head = source_locked;
    run.sources[item].conflict_baseline_entries = None;
    run.sources[item].phase = ItemPhase::Done(ItemOutcome::Squashed {
        commit,
        content_verified,
    });
    advance_after_item(state, run_id, item)
}

fn recheck(state: &mut AppState, run_id: WorktreeFinishRunId) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    // F5: never interrupt an in-flight AgentLaunching request with a Recheck transition.
    if matches!(
        run.sources.get(current).map(|s| &s.phase),
        Some(ItemPhase::AgentLaunching { .. })
    ) {
        return Vec::new();
    }
    let Some(site) = run
        .sources
        .get(current)
        .and_then(|s| s.phase.conflict_site().map(Path::to_path_buf))
    else {
        return Vec::new();
    };
    let verify_rebase_target = (run.mode == FinishMode::RebaseOnto)
        .then(|| run.target.locked_head.clone())
        .flatten();
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::RecheckSite {
        run_id,
        origin_repo_id: run.origin_repo_id,
        item: current,
        site,
        verify_rebase_target,
    })]
}

fn recheck_loaded(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    item: usize,
    snapshot: Result<FinishSnapshot, Error>,
    target_is_ancestor: Option<bool>,
    _rebased_range: Option<(CommitId, usize)>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    if current != item {
        return Vec::new();
    }

    let (site, stops, before_head, leaving_unconfirmed_agent) = match &run.sources[item].phase {
        ItemPhase::PausedConflict {
            site,
            stops,
            before_head,
            ..
        }
        | ItemPhase::AwaitingReview {
            site,
            stops,
            before_head,
            ..
        } => (site.clone(), *stops, before_head.clone(), false),
        ItemPhase::AgentDispatched {
            site,
            stops,
            before_head,
            session_id,
            agent_state,
            ..
        } => {
            let unconfirmed_agent = session_id.is_none()
                || !matches!(
                    agent_state,
                    Some(gitcomet_core::worktree_finish::AgentSessionState::Done)
                );
            (site.clone(), *stops, before_head.clone(), unconfirmed_agent)
        }
        // F5: ignore RecheckLoaded if a launch is currently in flight so it cannot reset
        // AgentLaunching back to PausedConflict and open a duplicate dispatch window.
        ItemPhase::AgentLaunching { .. } => return Vec::new(),
        ItemPhase::NeedsDecision { site, .. } => {
            let bh = match run.mode {
                FinishMode::MergeInto => run
                    .target
                    .locked_head
                    .clone()
                    .unwrap_or_else(|| CommitId("".into())),
                FinishMode::RebaseOnto => run.sources[item]
                    .locked_head
                    .clone()
                    .unwrap_or_else(|| CommitId("".into())),
            };
            (site.clone(), 1, bh, false)
        }
        _ => return Vec::new(),
    };

    let snap = match snapshot {
        Ok(snap) => snap,
        Err(err) => {
            run.sources[item].cleanup_disqualified =
                Some(CleanupBlocker::DisqualifiedByDrift(err.to_string()));
            run.sources[item].phase = ItemPhase::NeedsDecision {
                site,
                drift: DriftKind::Unexpected {
                    summary: format!("Failed to inspect worktree: {err}"),
                    snapshot: FinishSnapshot::default(),
                },
            };
            return Vec::new();
        }
    };

    if !snap.worktree_exists {
        run.sources[item].cleanup_disqualified = Some(CleanupBlocker::WorktreeMissing);
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::Unexpected {
                summary: "Worktree directory disappeared.".to_string(),
                snapshot: snap,
            },
        };
        return Vec::new();
    }

    let expected_branch = match run.mode {
        FinishMode::MergeInto => Some(run.target.branch.clone()),
        FinishMode::RebaseOnto => Some(run.sources[item].branch.clone()),
    };

    // Case 1: operation still in progress (including staged squash awaiting commit).
    let in_progress = snap.operation != FinishOperation::None || snap.has_staged_squash();
    if in_progress {
        if !snap.unmerged.is_empty() || !snap.conflict_marker_paths.is_empty() {
            if leaving_unconfirmed_agent {
                // F5: manual Recheck while a previously launched agent is still running or lacks
                // an exact session receipt must lock out fresh duplicate launches.
                run.sources[item].agent_receipt_unknown = true;
            }
            run.sources[item].phase = ItemPhase::PausedConflict {
                site,
                stops,
                before_head,
                snapshot: snap,
            };
        } else {
            // T-D1: conflicts resolved & no residual markers -> AwaitingReview.
            run.sources[item].phase = ItemPhase::AwaitingReview {
                site,
                stops,
                before_head,
                snapshot: snap,
            };
        }
        return Vec::new();
    }

    // Case 2: operation ended. Check branch identity first (T-D4).
    if snap.detached || snap.branch.as_deref() != expected_branch.as_deref() {
        run.sources[item].cleanup_disqualified = Some(CleanupBlocker::BranchChangedOrDetached);
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::Unexpected {
                summary: format!(
                    "Expected branch {:?}, found {:?} (detached={}).",
                    expected_branch, snap.branch, snap.detached
                ),
                snapshot: snap,
            },
        };
        return Vec::new();
    }

    // Case 2a: HEAD equals before_head -> Aborted (T-D3: never NothingNew!).
    if snap.head.as_ref() == Some(&before_head) {
        run.sources[item].phase = ItemPhase::NeedsDecision {
            site,
            drift: DriftKind::Aborted {
                before_head,
                snapshot: snap,
            },
        };
        return Vec::new();
    }

    // Case 2b: HEAD advanced with clean tracked state and expected commit shape -> AdoptCommit (T-D2/T-A4).
    if let Some(after_head) = snap.head.clone()
        && snap.is_clean_tracked()
    {
        let expected_shape = match (run.mode, run.sources[item].squash) {
            (FinishMode::MergeInto, false) => {
                snap.head_parents.len() >= 2
                    && run.sources[item]
                        .locked_head
                        .as_ref()
                        .is_some_and(|src| snap.head_parents.contains(src))
            }
            (FinishMode::MergeInto, true) => snap.head_parents == vec![before_head.clone()],
            (FinishMode::RebaseOnto, _) => target_is_ancestor == Some(true),
        };
        if expected_shape {
            run.sources[item].phase = ItemPhase::NeedsDecision {
                site,
                drift: DriftKind::AdoptCommit {
                    before_head,
                    after_head,
                    snapshot: snap,
                },
            };
            return Vec::new();
        }
    }

    // Case 2c: unexpected HEAD or dirty tracked changes (T-D4).
    run.sources[item].cleanup_disqualified = Some(CleanupBlocker::DisqualifiedByDrift(
        "Unexpected HEAD or working tree state after pause.".to_string(),
    ));
    run.sources[item].phase = ItemPhase::NeedsDecision {
        site,
        drift: DriftKind::Unexpected {
            summary: "HEAD moved unexpectedly or uncommitted changes remain.".to_string(),
            snapshot: snap,
        },
    };
    Vec::new()
}

fn continue_item(state: &mut AppState, run_id: WorktreeFinishRunId) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    let ItemPhase::AwaitingReview { site, snapshot, .. } = &run.sources[current].phase else {
        return Vec::new();
    };
    if !snapshot.unmerged.is_empty() || !snapshot.conflict_marker_paths.is_empty() {
        return Vec::new();
    }
    let site = site.clone();
    let (step, action, verify_rebase_target) = match (run.mode, run.sources[current].squash) {
        (FinishMode::MergeInto, false) => (
            FinishStep::ConcludeMerge,
            FinishStepAction::ConcludeMerge { message: None },
            None,
        ),
        (FinishMode::MergeInto, true) => (
            FinishStep::SquashCommit,
            FinishStepAction::SquashCommit {
                message: resolved_squash_message(run, current),
            },
            None,
        ),
        (FinishMode::RebaseOnto, _) => {
            let target_head = run.target.locked_head.clone().expect("target locked head");
            (
                FinishStep::RebaseContinue,
                FinishStepAction::RebaseContinue {
                    target_head: target_head.clone(),
                },
                Some(target_head),
            )
        }
    };

    run.sources[current].user_reviewed_resolution = true;
    let guard = build_step_guard(run, current, &step);
    run.phase = RunPhase::Running { current };
    run.sources[current].phase = ItemPhase::Running(step.clone());
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
        run_id,
        origin_repo_id: run.origin_repo_id,
        item: current,
        step,
        site,
        guard,
        action,
        verify_rebase_target,
        auth: None,
    })]
}

fn skip_item(state: &mut AppState, run_id: WorktreeFinishRunId) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    // F5: never mutate/abort the conflict site while an agent launch is in flight.
    if matches!(run.sources[current].phase, ItemPhase::AgentLaunching { .. }) {
        return Vec::new();
    }
    let paused_head = match &run.sources[current].phase {
        ItemPhase::PausedConflict { snapshot, .. }
        | ItemPhase::AgentDispatched { snapshot, .. }
        | ItemPhase::AwaitingReview { snapshot, .. } => Some(snapshot.head.clone()),
        _ => None,
    };
    let Some(expected_head) = paused_head else {
        run.sources[current].phase = ItemPhase::Skipped(SkipReason::UserSkipped);
        return advance_after_item(state, run_id, current);
    };

    let site = execution_site(run, current);
    let safety = AbortSafetyBaseline {
        expected_head,
        baseline_entries: run.sources[current].conflict_baseline_entries.clone(),
    };
    let action = abort_action_for(run.mode, run.sources[current].squash, safety);
    let guard = build_step_guard(run, current, &FinishStep::AbortItem);
    run.phase = RunPhase::Running { current };
    run.sources[current].phase = ItemPhase::Running(FinishStep::AbortItem);
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
        run_id,
        origin_repo_id: run.origin_repo_id,
        item: current,
        step: FinishStep::AbortItem,
        site,
        guard,
        action,
        verify_rebase_target: None,
        auth: None,
    })]
}

fn abort_remaining(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    keep_conflict_state: bool,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let current = match run.phase {
        RunPhase::Paused { current } | RunPhase::Running { current } => current,
        RunPhase::Previewing | RunPhase::AwaitingConfirm { .. } => {
            if run.is_initial_draft() {
                run.phase = RunPhase::Aborted;
                return Vec::new();
            }
            // F8: aborting during mid-batch RefreshPreview must skip remaining items and still
            // run batch-end cleanup for already completed items and any created temp worktree.
            let start = run.resume_from;
            for idx in start..run.sources.len() {
                if !run.sources[idx].phase.is_terminal() {
                    run.sources[idx].phase = ItemPhase::Skipped(SkipReason::AbortedByUser);
                }
            }
            return begin_cleanup_check(run);
        }
        _ => return Vec::new(),
    };

    if !keep_conflict_state
        && matches!(run.sources[current].phase, ItemPhase::AgentLaunching { .. })
    {
        return Vec::new();
    }

    for idx in (current + 1)..run.sources.len() {
        if matches!(run.sources[idx].phase, ItemPhase::Queued) {
            run.sources[idx].phase = ItemPhase::Skipped(SkipReason::AbortedByUser);
        }
    }

    let paused_head = match &run.sources[current].phase {
        ItemPhase::PausedConflict { snapshot, .. }
        | ItemPhase::AgentDispatched { snapshot, .. }
        | ItemPhase::AwaitingReview { snapshot, .. } => Some(snapshot.head.clone()),
        _ => None,
    };
    if let Some(expected_head) = paused_head
        && !keep_conflict_state
    {
        let site = execution_site(run, current);
        let safety = AbortSafetyBaseline {
            expected_head,
            baseline_entries: run.sources[current].conflict_baseline_entries.clone(),
        };
        let action = abort_action_for(run.mode, run.sources[current].squash, safety);
        let guard = build_step_guard(run, current, &FinishStep::AbortItem);
        run.phase = RunPhase::Running { current };
        run.sources[current].phase = ItemPhase::Running(FinishStep::AbortItem);
        return vec![Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
            run_id,
            origin_repo_id: run.origin_repo_id,
            item: current,
            step: FinishStep::AbortItem,
            site,
            guard,
            action,
            verify_rebase_target: None,
            auth: None,
        })];
    }

    if !run.sources[current].phase.is_terminal() {
        run.sources[current].phase = ItemPhase::Skipped(SkipReason::AbortedByUser);
    }
    begin_cleanup_check(run)
}

fn load_agents(state: &mut AppState, run_id: WorktreeFinishRunId) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    let site = execution_site(run, current);
    run.agent_dispatch.installed = Loadable::Loading;
    run.agent_dispatch.last_error = None;
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::LoadAgents {
        run_id,
        site,
    })]
}

fn agents_loaded(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    availability: AgentDispatchAvailability,
    agents: Result<Vec<InstalledAgent>, Error>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    run.agent_dispatch.availability = Some(availability);
    match agents {
        Ok(list) => {
            run.agent_dispatch.installed = Loadable::Ready(list);
            run.agent_dispatch.last_error = None;
        }
        Err(err) => {
            let msg = err.to_string();
            run.agent_dispatch.installed = Loadable::Error(msg.clone());
            run.agent_dispatch.last_error = Some(msg);
        }
    }
    Vec::new()
}

fn dispatch_agent(
    inputs: &ReducerInputs,
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    agent_id: String,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    // F5: refuse duplicate launch if a previous launch receipt was unknown/ambiguous or an agent
    // was already dispatched for this conflict stop.
    if run.sources[current].agent_receipt_unknown {
        run.agent_dispatch.last_error = Some(
            "Previous agent launch receipt is uncertain or active; inspect workspace or recheck Git instead of launching another agent."
                .to_string(),
        );
        return Vec::new();
    }
    let ItemPhase::PausedConflict {
        site,
        stops,
        before_head,
        snapshot,
    } = run.sources[current].phase.clone()
    else {
        return Vec::new();
    };
    let op_label = match (run.mode, run.sources[current].squash) {
        (FinishMode::MergeInto, false) => "merge",
        (FinishMode::MergeInto, true) => "squash merge",
        (FinishMode::RebaseOnto, _) => "rebase",
    };
    let prompt = conflict_resolution_prompt(&site, op_label, &snapshot.unmerged);
    let request_id = run.next_agent_request_id;
    run.next_agent_request_id = run.next_agent_request_id.saturating_add(1);
    run.sources[current].dispatched_agent = true;
    run.agent_dispatch.last_error = None;
    // F5: transition synchronously into AgentLaunching BEFORE returning effect so re-entrant
    // DispatchAgent calls are rejected immediately.
    run.sources[current].phase = ItemPhase::AgentLaunching {
        site: site.clone(),
        stops,
        before_head,
        agent_id: agent_id.clone(),
        request_id,
        started_at: inputs.now(),
        snapshot,
    };
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::LaunchAgent {
        run_id,
        item: current,
        site,
        agent_id,
        request_id,
        prompt,
        retry_operation_id: None,
    })]
}

fn agent_launched(
    inputs: &ReducerInputs,
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    item: usize,
    agent_id: String,
    request_id: u64,
    result: Result<AgentLaunch, Error>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    if current != item {
        return Vec::new();
    }
    let ItemPhase::AgentLaunching {
        site,
        stops,
        before_head,
        request_id: expected_request_id,
        snapshot,
        ..
    } = run.sources[item].phase.clone()
    else {
        return Vec::new();
    };
    if expected_request_id != request_id {
        return Vec::new();
    }

    match result {
        Ok(launch) => {
            // D5 / F5: only record exact session_id returned in the launch receipt. If session_id
            // is missing, mark receipt unknown so no duplicate launch can be issued.
            let receipt_missing_session = launch.session_id.is_none();
            if receipt_missing_session {
                run.sources[item].agent_receipt_unknown = true;
                run.sources[item].pending_retry_operation_id = launch.operation_id.clone();
            }
            let detail = launch.detail.or_else(|| {
                receipt_missing_session.then(|| {
                    "Launch returned no exact session ID; inspect workspace or recheck Git instead of launching another agent."
                        .to_string()
                })
            });
            run.sources[item].phase = ItemPhase::AgentDispatched {
                site,
                stops,
                before_head,
                agent_id,
                session_id: launch.session_id,
                operation_id: launch.operation_id,
                launched_at: inputs.now(),
                last_polled_at: None,
                agent_state: None,
                last_assistant_message: detail,
                timed_out: false,
                snapshot,
            };
        }
        Err(err) => {
            let err_str = err.to_string();
            let definitely_not_started = err_str.contains("Orca CLI executable was not found");
            if !definitely_not_started {
                // F5: launch may have succeeded on the Orca host before failing/losing reply;
                // lock out fresh re-dispatch and record any `--retry-launch <operationId>`.
                run.sources[item].agent_receipt_unknown = true;
                run.sources[item].pending_retry_operation_id =
                    super::super::orca_dispatcher::extract_retry_operation_id(&err_str);
                run.agent_dispatch.last_error = Some(format!(
                    "Agent launch receipt uncertain ({err_str}). Fresh re-dispatch is blocked; inspect workspace or recheck Git."
                ));
            } else {
                // T-A5: definitely not launched -> show error and fall back to manual resolution.
                run.agent_dispatch.last_error = Some(err_str);
            }
            run.sources[item].phase = ItemPhase::PausedConflict {
                site,
                stops,
                before_head,
                snapshot,
            };
        }
    }
    Vec::new()
}

fn poll_agent(
    inputs: &ReducerInputs,
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    let ItemPhase::AgentDispatched {
        site,
        launched_at,
        timed_out,
        ..
    } = &mut run.sources[current].phase
    else {
        return Vec::new();
    };
    if let Ok(elapsed) = inputs.now().duration_since(*launched_at)
        && elapsed >= AGENT_POLL_TIMEOUT
    {
        // T-A7: mark unresponsive without killing the agent; open manual fallback.
        *timed_out = true;
        return Vec::new();
    }
    let site = site.clone();
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::PollAgent {
        run_id,
        item: current,
        site,
    })]
}

fn agent_polled(
    inputs: &ReducerInputs,
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    item: usize,
    sessions: Result<Vec<AgentSession>, Error>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    if current != item {
        return Vec::new();
    }
    let origin_repo_id = run.origin_repo_id;
    let verify_rebase_target = (run.mode == FinishMode::RebaseOnto)
        .then(|| run.target.locked_head.clone())
        .flatten();
    let ItemPhase::AgentDispatched {
        site,
        session_id,
        launched_at,
        last_polled_at,
        agent_state,
        last_assistant_message,
        timed_out,
        ..
    } = &mut run.sources[item].phase
    else {
        return Vec::new();
    };

    *last_polled_at = Some(inputs.now());
    if let Ok(elapsed) = inputs.now().duration_since(*launched_at)
        && elapsed >= AGENT_POLL_TIMEOUT
    {
        *timed_out = true;
    }

    let Ok(sessions) = sessions else {
        return Vec::new();
    };
    // D5: match strictly by exact session_id; never guess by latest same-agent session.
    let Some(expected_id) = session_id.as_deref() else {
        return Vec::new();
    };
    let Some(matched) = sessions.iter().find(|s| s.session_id == expected_id) else {
        return Vec::new();
    };

    *agent_state = Some(matched.state.clone());
    if let Some(msg) = &matched.last_assistant_message {
        *last_assistant_message = Some(msg.clone());
    }

    if matched.state.triggers_recheck() {
        return vec![Effect::WorktreeFinish(WorktreeFinishEffect::RecheckSite {
            run_id,
            origin_repo_id,
            item,
            site: site.clone(),
            verify_rebase_target,
        })];
    }
    Vec::new()
}

fn switch_to_manual(state: &mut AppState, run_id: WorktreeFinishRunId) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    let ItemPhase::AgentDispatched {
        site,
        stops,
        before_head,
        snapshot,
        ..
    } = run.sources[current].phase.clone()
    else {
        return Vec::new();
    };
    // F5: the previously launched agent is NOT killed when switching to manual; block launching
    // a second overlapping agent on the same conflict stop.
    run.sources[current].agent_receipt_unknown = true;
    run.sources[current].phase = ItemPhase::PausedConflict {
        site,
        stops,
        before_head,
        snapshot,
    };
    Vec::new()
}

fn resolve_drift(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    choice: DriftResolution,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    let ItemPhase::NeedsDecision { site, drift } = run.sources[current].phase.clone() else {
        return Vec::new();
    };

    match (drift, choice) {
        (
            DriftKind::AdoptCommit {
                after_head,
                snapshot,
                ..
            },
            DriftResolution::AdoptCommit,
        ) => {
            // T-D2 / T-A4: adopt existing commit without committing again!
            let source_locked = run.sources[current].locked_head.clone();
            run.sources[current].user_reviewed_resolution = true;
            let outcome = match (run.mode, run.sources[current].squash) {
                (FinishMode::MergeInto, false) => {
                    run.target.locked_head = Some(after_head.clone());
                    run.sources[current].result_head = source_locked;
                    ItemOutcome::Merged { commit: after_head }
                }
                (FinishMode::MergeInto, true) => {
                    run.target.locked_head = Some(after_head.clone());
                    run.sources[current].result_head = source_locked;
                    let verified = if !run.sources[current].had_conflict
                        && run.sources[current].expected_merged_tree.is_some()
                        && run.sources[current].expected_merged_tree == snapshot.head_tree
                    {
                        SquashContentCheck::Verified
                    } else {
                        SquashContentCheck::ReviewedResolution
                    };
                    ItemOutcome::Squashed {
                        commit: after_head,
                        content_verified: verified,
                    }
                }
                (FinishMode::RebaseOnto, false) => {
                    run.sources[current].result_head = Some(after_head.clone());
                    ItemOutcome::Rebased {
                        new_head: after_head,
                        commits: 1,
                    }
                }
                (FinishMode::RebaseOnto, true) => {
                    run.sources[current].result_head = Some(after_head.clone());
                    if run
                        .target
                        .locked_head
                        .as_ref()
                        .is_some_and(|t| snapshot.head_parents == vec![t.clone()])
                    {
                        ItemOutcome::RebasedSquashed {
                            new_head: after_head,
                        }
                    } else {
                        ItemOutcome::Rebased {
                            new_head: after_head,
                            commits: 1,
                        }
                    }
                }
            };
            run.sources[current].phase = ItemPhase::Done(outcome);
            advance_after_item(state, run_id, current)
        }
        (DriftKind::AdoptCommit { before_head, .. }, DriftResolution::RollbackToBeforeHead) => {
            let step = FinishStep::RollbackToHead {
                head: before_head.clone(),
            };
            let guard = build_step_guard(run, current, &step);
            run.phase = RunPhase::Running { current };
            run.sources[current].phase = ItemPhase::Running(step.clone());
            vec![Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
                run_id,
                origin_repo_id: run.origin_repo_id,
                item: current,
                step,
                site,
                guard,
                action: FinishStepAction::RollbackToHead { head: before_head },
                verify_rebase_target: None,
                auth: None,
            })]
        }
        (DriftKind::Aborted { .. }, DriftResolution::RequeueItem) => {
            // T-D3: re-run this item from preflight. Preserve `agent_receipt_unknown` if a prior
            // launch had an uncertain receipt so no overlapping duplicate agent can be dispatched.
            run.sources[current].had_conflict = false;
            run.sources[current].user_reviewed_resolution = false;
            run.sources[current].conflict_baseline_entries = None;
            start_item_preflight(run, current)
        }
        (DriftKind::SourceMoved { .. }, DriftResolution::KeepLockedHead)
            if run.mode == FinishMode::MergeInto =>
        {
            // T-Q4: merge locked head and cancel cleanup so new commits in source are never lost.
            run.sources[current].cleanup_disqualified =
                Some(CleanupBlocker::HeadChangedAfterFinish);
            dispatch_primary_step(run, current)
        }
        (
            DriftKind::SourceMoved { .. } | DriftKind::TargetMoved { .. },
            DriftResolution::RefreshPreview,
        ) => {
            // F8: re-preview remaining items from `current` and require a fresh Confirm + fingerprint;
            // NEVER adopt new HEADs and execute writes without showing the updated preview first.
            let origin_repo_id = run.origin_repo_id;
            let target_branch = run.target.branch.clone();
            let mode = run.mode;
            run.resume_from = current;
            run.preview_blockers.clear();
            run.preview_warnings.clear();
            run.last_error = None;
            let mut remaining_requests = Vec::with_capacity(run.sources.len() - current);
            for src in run.sources.iter_mut().skip(current) {
                src.locked_head = None;
                src.initial_snapshot = None;
                src.expected_merged_tree = None;
                src.preview = Loadable::Loading;
                src.phase = ItemPhase::Queued;
                src.had_conflict = false;
                src.conflict_stops = 0;
                src.user_reviewed_resolution = false;
                src.conflict_baseline_entries = None;
                src.pending_squash_range = None;
                src.cleanup_disqualified = None;
                remaining_requests.push(FinishSourceRequest {
                    path: src.path.clone(),
                    branch: src.branch.clone(),
                    squash: src.squash,
                    remove_after: src.remove_after,
                    squash_message: src.squash_message.clone(),
                });
            }
            run.phase = RunPhase::Previewing;
            vec![Effect::WorktreeFinish(WorktreeFinishEffect::LoadPreview {
                run_id,
                origin_repo_id,
                start_index: current,
                target_branch,
                mode,
                sources: remaining_requests,
            })]
        }
        (_, DriftResolution::SkipItem) => {
            run.sources[current].phase = ItemPhase::Skipped(SkipReason::SourceMovedSkipped);
            advance_after_item(state, run_id, current)
        }
        (_, DriftResolution::AbortRemaining) => abort_remaining(state, run_id, true),
        _ => Vec::new(),
    }
}

fn retry_credential(state: &mut AppState, run_id: WorktreeFinishRunId) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let RunPhase::Paused { current } = run.phase else {
        return Vec::new();
    };
    let ItemPhase::AwaitingCredential { step, site, .. } = run.sources[current].phase.clone()
    else {
        return Vec::new();
    };
    let action = match &step {
        FinishStep::Merge => FinishStepAction::Merge {
            source_head: run.sources[current]
                .locked_head
                .clone()
                .expect("locked head"),
            message: default_merge_branch_message(&run.sources[current].branch, &run.target.branch),
        },
        FinishStep::SquashCommit => FinishStepAction::SquashCommit {
            message: resolved_squash_message(run, current),
        },
        FinishStep::ConcludeMerge => FinishStepAction::ConcludeMerge { message: None },
        FinishStep::RebaseContinue => FinishStepAction::RebaseContinue {
            target_head: run.target.locked_head.clone().expect("target head"),
        },
        FinishStep::Rebase => FinishStepAction::Rebase {
            target_head: run.target.locked_head.clone().expect("target head"),
        },
        FinishStep::SquashRange => {
            let Some((oldest, expected_head)) = run.sources[current].pending_squash_range.clone()
            else {
                return Vec::new();
            };
            FinishStepAction::SquashRange {
                oldest,
                expected_head,
                message: resolved_squash_message(run, current),
            }
        }
        _ => return Vec::new(),
    };
    let verify_rebase_target = matches!(step, FinishStep::Rebase | FinishStep::RebaseContinue)
        .then(|| run.target.locked_head.clone())
        .flatten();
    let guard = build_step_guard(run, current, &step);
    run.phase = RunPhase::Running { current };
    run.sources[current].phase = ItemPhase::Running(step.clone());
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
        run_id,
        origin_repo_id: run.origin_repo_id,
        item: current,
        step,
        site,
        guard,
        action,
        verify_rebase_target,
        auth: None,
    })]
}

fn advance_after_item(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    completed_item: usize,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    let next = completed_item + 1;
    if next < run.sources.len() {
        start_item_preflight(run, next)
    } else {
        begin_cleanup_check(run)
    }
}

fn begin_cleanup_check(run: &mut WorktreeFinishRun) -> Vec<Effect> {
    let mut to_check = Vec::new();
    for (idx, source) in run.sources.iter_mut().enumerate() {
        if !source.remove_after {
            source.cleanup = CleanupOutcome::NotRequested;
            continue;
        }
        if !matches!(source.phase, ItemPhase::Done(_)) {
            // T-C7: skipped or failed items are never cleaned up.
            source.cleanup = CleanupOutcome::NotRequested;
            continue;
        }
        if let Some(reason) = source.cleanup_disqualified.clone() {
            source.cleanup = CleanupOutcome::Kept(reason);
            continue;
        }
        source.cleanup = CleanupOutcome::Pending;
        to_check.push((idx, source.path.clone()));
    }

    if to_check.is_empty() {
        return finish_or_clean_temp_worktree(run);
    }

    run.phase = RunPhase::CleanupCheck;
    vec![Effect::WorktreeFinish(WorktreeFinishEffect::CheckCleanup {
        run_id: run.id,
        origin_repo_id: run.origin_repo_id,
        items: to_check,
    })]
}

fn cleanup_checked(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    results: Vec<SourceCleanupCheckResult>,
    orca_unavailable: bool,
    live_terminal_paths: Vec<PathBuf>,
) -> Vec<Effect> {
    let global_terminals = state.running_terminal_paths.clone();
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    if run.phase != RunPhase::CleanupCheck {
        return Vec::new();
    }

    // F9: merge run-local, global AppState, and live worker-read terminal paths so any terminal
    // opened after Confirm is guaranteed to block cleanup.
    let mut running_terminals: HashSet<PathBuf> = run
        .running_terminal_paths
        .iter()
        .chain(global_terminals.iter())
        .chain(live_terminal_paths.iter())
        .cloned()
        .map(canonicalize_or_original)
        .collect();
    // If live_terminal_paths is non-empty, keep run snapshot in sync as well.
    if !live_terminal_paths.is_empty() {
        run.running_terminal_paths = live_terminal_paths;
    }
    let _ = &mut running_terminals;

    let target_host_canonical = run
        .target
        .host
        .path()
        .map(|p| canonicalize_or_original(p.to_path_buf()));

    let expected_common_dir = run.common_dir.clone();
    let mut remove_effects = Vec::new();
    for check in results {
        let Some(source) = run.sources.get_mut(check.index) else {
            continue;
        };
        if source.cleanup != CleanupOutcome::Pending {
            continue;
        }
        let blocker = evaluate_cleanup_safety(
            source,
            &check,
            &running_terminals,
            target_host_canonical.as_deref(),
        );
        if let Some(reason) = blocker {
            source.cleanup = CleanupOutcome::Kept(reason);
            continue;
        }

        if orca_unavailable && matches!(check.active_sessions, Err(AgentCheckStatus::OrcaNotFound))
        {
            run.cleanup_agent_check_unverified = true;
            source
                .cleanup_notes
                .push("Orca unavailable; external agent occupancy was not checked.".to_string());
        }

        let Some(expected_head) = source.result_head.clone() else {
            source.cleanup = CleanupOutcome::Kept(CleanupBlocker::ResolutionUnverified);
            continue;
        };
        let expected_ignored_fingerprint = source
            .initial_snapshot
            .as_ref()
            .and_then(|s| s.ignored_fingerprint.clone());
        remove_effects.push(Effect::WorktreeFinish(
            WorktreeFinishEffect::RemoveWorktree {
                run_id,
                origin_repo_id: run.origin_repo_id,
                item: check.index,
                path: source.path.clone(),
                expected_common_dir: expected_common_dir.clone(),
                expected_branch: source.branch.clone(),
                expected_head,
                expected_ignored_fingerprint,
            },
        ));
    }

    if remove_effects.is_empty() {
        return finish_or_clean_temp_worktree(run);
    }

    run.phase = RunPhase::CleaningUp;
    remove_effects
}

fn evaluate_cleanup_safety(
    source: &FinishSource,
    check: &SourceCleanupCheckResult,
    running_terminals: &HashSet<PathBuf>,
    target_host_canonical: Option<&Path>,
) -> Option<CleanupBlocker> {
    if let Some(reason) = &source.cleanup_disqualified {
        return Some(reason.clone());
    }
    // D9: if conflict resolution was never explicitly confirmed by the user, keep worktree.
    if matches!(
        &source.phase,
        ItemPhase::Done(ItemOutcome::Squashed {
            content_verified: SquashContentCheck::ReviewedResolution,
            ..
        })
    ) && !source.user_reviewed_resolution
    {
        return Some(CleanupBlocker::ResolutionUnverified);
    }

    let snap = match &check.snapshot {
        Ok(s) => s,
        Err(err) => return Some(CleanupBlocker::GitRefused(err.to_string())),
    };
    if !snap.worktree_exists {
        return Some(CleanupBlocker::WorktreeMissing);
    }
    if snap.is_primary_worktree {
        return Some(CleanupBlocker::PrimaryWorktree);
    }
    if snap.is_bare {
        return Some(CleanupBlocker::BareWorktree);
    }
    let canonical_src = canonicalize_or_original(source.path.clone());
    if target_host_canonical.is_some_and(|t| t == canonical_src.as_path()) {
        return Some(CleanupBlocker::InProgressOperation);
    }
    if snap.head != source.result_head {
        return Some(CleanupBlocker::HeadChangedAfterFinish);
    }
    if snap.detached || snap.branch.as_deref() != Some(source.branch.as_str()) {
        return Some(CleanupBlocker::BranchChangedOrDetached);
    }
    if !snap.is_completely_clean() {
        return Some(CleanupBlocker::DirtyOrUntracked);
    }
    if snap.operation != FinishOperation::None || snap.has_squash_msg {
        return Some(CleanupBlocker::InProgressOperation);
    }
    if snap.locked {
        return Some(CleanupBlocker::WorktreeLocked);
    }
    if snap.has_initialized_submodules {
        return Some(CleanupBlocker::HasSubmodules);
    }
    // D3 / F2: ignored paths AND file contents must have been reliably fingerprinted at preview
    // and verified unchanged at cleanup re-check; any change or unverifiable state keeps the folder.
    match &source.initial_snapshot {
        None => return Some(CleanupBlocker::IgnoredFilesChangedOrUnverified),
        Some(initial) => {
            if snap.ignored_count != initial.ignored_count {
                return Some(CleanupBlocker::IgnoredFilesChangedOrUnverified);
            }
            match (&initial.ignored_fingerprint, &snap.ignored_fingerprint) {
                (
                    Some(IgnoredFilesFingerprint::Verified {
                        entry_count: init_c,
                        digest: init_d,
                    }),
                    Some(IgnoredFilesFingerprint::Verified {
                        entry_count: cur_c,
                        digest: cur_d,
                    }),
                ) if init_c == cur_c && init_d == cur_d => {}
                _ => return Some(CleanupBlocker::IgnoredFilesChangedOrUnverified),
            }
        }
    }
    if running_terminals.contains(&canonical_src) {
        return Some(CleanupBlocker::OpenGitCometTerminal);
    }
    // D4: check Orca session occupancy conservatively.
    match &check.active_sessions {
        Ok(sessions) => {
            if sessions.iter().any(|s| s.state.is_active()) {
                return Some(CleanupBlocker::ActiveOrcaAgent);
            }
        }
        Err(AgentCheckStatus::QueryFailed(_)) => {
            return Some(CleanupBlocker::OrcaSessionUnverifiable);
        }
        Err(AgentCheckStatus::OrcaNotFound) => {
            if source.dispatched_agent || !source.initial_active_sessions.is_empty() {
                return Some(CleanupBlocker::OrcaSessionUnverifiable);
            }
        }
    }
    None
}

fn worktree_removed(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    item: usize,
    result: Result<gitcomet_core::services::CommandOutput, Error>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    if run.phase != RunPhase::CleaningUp {
        return Vec::new();
    }
    if let Some(source) = run.sources.get_mut(item) {
        match result {
            Ok(_) => source.cleanup = CleanupOutcome::Removed,
            Err(err) => {
                source.cleanup = CleanupOutcome::Kept(CleanupBlocker::GitRefused(err.to_string()))
            }
        }
    }
    let any_pending = run
        .sources
        .iter()
        .any(|s| s.cleanup == CleanupOutcome::Pending);
    if any_pending {
        return Vec::new();
    }
    let mut effects = finish_or_clean_temp_worktree(run);
    effects.extend(refresh_effects_for_run(state, run_id));
    effects
}

fn finish_or_clean_temp_worktree(run: &mut WorktreeFinishRun) -> Vec<Effect> {
    if let TargetHost::Temporary {
        path,
        created: true,
    } = &run.target.host
    {
        let temp_path = path.clone();
        run.phase = RunPhase::CleaningUp;
        return vec![Effect::WorktreeFinish(
            WorktreeFinishEffect::CleanupTempWorktree {
                run_id: run.id,
                origin_repo_id: run.origin_repo_id,
                temp_path,
            },
        )];
    }
    finalize_run_phase(run);
    let mut repo_ids = vec![run.origin_repo_id];
    if let TargetHost::OpenTab { repo_id, .. } = run.target.host
        && !repo_ids.contains(&repo_id)
    {
        repo_ids.push(repo_id);
    }
    let mut effects = Vec::new();
    for repo_id in repo_ids {
        effects.push(Effect::LoadWorktrees { repo_id });
        effects.push(Effect::References(ReferenceEffect::LoadBranches {
            repo_id,
        }));
        effects.push(Effect::Status(StatusEffect::LoadStatus { repo_id }));
        effects.push(Effect::LoadRebaseAndMergeState { repo_id });
    }
    effects
}

fn temp_worktree_cleaned(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
    removed: bool,
    note: Option<String>,
) -> Vec<Effect> {
    let Some(run) = find_run_mut(state, run_id) else {
        return Vec::new();
    };
    if removed && let TargetHost::Temporary { created, .. } = &mut run.target.host {
        *created = false;
    }
    run.temp_worktree_kept_note = note;
    finalize_run_phase(run);
    refresh_effects_for_run(state, run_id)
}

fn finalize_run_phase(run: &mut WorktreeFinishRun) {
    let was_aborted = run
        .sources
        .iter()
        .any(|s| matches!(s.phase, ItemPhase::Skipped(SkipReason::AbortedByUser)));
    run.phase = if was_aborted {
        RunPhase::Aborted
    } else {
        RunPhase::Finished
    };
}

fn refresh_effects_for_run(state: &AppState, run_id: WorktreeFinishRunId) -> Vec<Effect> {
    let Some(run) = state.worktree_finish_runs.iter().find(|r| r.id == run_id) else {
        return Vec::new();
    };
    let origin_repo_id = run.origin_repo_id;
    let mut repo_ids = vec![origin_repo_id];
    if let TargetHost::OpenTab { repo_id, .. } = run.target.host
        && !repo_ids.contains(&repo_id)
    {
        repo_ids.push(repo_id);
    }
    let mut effects = Vec::new();
    for repo_id in repo_ids {
        effects.push(Effect::LoadWorktrees { repo_id });
        effects.push(Effect::References(ReferenceEffect::LoadBranches {
            repo_id,
        }));
        effects.push(Effect::Status(StatusEffect::LoadStatus { repo_id }));
        effects.push(Effect::LoadRebaseAndMergeState { repo_id });
    }
    effects
}

fn fail_item(
    run: &mut WorktreeFinishRun,
    item: usize,
    message: &str,
    output: Option<String>,
) -> Vec<Effect> {
    run.sources[item].phase = ItemPhase::Failed {
        message: message.to_string(),
        output,
    };
    run.phase = RunPhase::Paused { current: item };
    Vec::new()
}

fn execution_site(run: &WorktreeFinishRun, item: usize) -> PathBuf {
    match run.mode {
        FinishMode::MergeInto => run
            .target
            .host
            .path()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| run.sources[item].path.clone()),
        FinishMode::RebaseOnto => run.sources[item].path.clone(),
    }
}

fn abort_action_for(
    mode: FinishMode,
    squash: bool,
    safety: AbortSafetyBaseline,
) -> FinishStepAction {
    match (mode, squash) {
        (FinishMode::MergeInto, false) => FinishStepAction::AbortMerge { safety },
        (FinishMode::MergeInto, true) => FinishStepAction::AbortSquashMerge { safety },
        (FinishMode::RebaseOnto, _) => FinishStepAction::AbortRebase { safety },
    }
}

fn build_step_guard(run: &WorktreeFinishRun, item: usize, step: &FinishStep) -> StepSiteGuard {
    let expected_branch = match run.mode {
        FinishMode::MergeInto => run.target.branch.clone(),
        FinishMode::RebaseOnto => run.sources[item].branch.clone(),
    };
    let allow_detached_rebase = matches!(step, FinishStep::RebaseContinue)
        || (matches!(step, FinishStep::AbortItem) && run.mode == FinishMode::RebaseOnto);
    let expected_head = match step {
        FinishStep::Merge
        | FinishStep::SquashMerge
        | FinishStep::SquashCommit
        | FinishStep::ConcludeMerge => run.target.locked_head.clone(),
        FinishStep::Rebase => run.sources[item].locked_head.clone(),
        FinishStep::SquashRange => run.sources[item]
            .pending_squash_range
            .as_ref()
            .map(|(_, head)| head.clone()),
        FinishStep::AbortItem if run.mode == FinishMode::MergeInto => {
            run.target.locked_head.clone()
        }
        FinishStep::AbortItem
        | FinishStep::RebaseContinue
        | FinishStep::RollbackToHead { .. }
        | FinishStep::PrepareTempWorktree
        | FinishStep::PreflightItem => None,
    };
    let require_clean_tracked = matches!(
        step,
        FinishStep::Merge | FinishStep::SquashMerge | FinishStep::Rebase | FinishStep::SquashRange
    );
    let require_no_operation = require_clean_tracked;
    StepSiteGuard {
        expected_common_dir: run.common_dir.clone(),
        expected_branch,
        expected_head,
        allow_detached_rebase,
        require_clean_tracked,
        require_no_operation,
    }
}

fn resolved_squash_message(run: &WorktreeFinishRun, item: usize) -> String {
    if let Some(msg) = run.sources[item]
        .squash_message
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return msg.to_string();
    }
    let subjects = match &run.sources[item].preview {
        Loadable::Ready(p) => p.commit_subjects.as_slice(),
        _ => &[],
    };
    default_squash_merge_message(&run.sources[item].branch, &run.target.branch, subjects)
}

fn find_run_mut(
    state: &mut AppState,
    run_id: WorktreeFinishRunId,
) -> Option<&mut WorktreeFinishRun> {
    state
        .worktree_finish_runs
        .iter_mut()
        .find(|run| run.id == run_id)
}
