use super::*;
use crate::model::{
    AuthRetryOperation, CleanupBlocker, CleanupOutcome, DriftKind, DriftResolution, FinishMode,
    FinishSourceRequest, FinishStep, ItemOutcome, ItemPhase, Loadable, RunPhase, SkipReason,
    SquashContentCheck, TargetHost, WorktreeFinishRun, WorktreeFinishRunId,
};
use crate::msg::{
    AgentCheckStatus, Effect, FinishStepAction, InternalMsg, Msg, SourceCleanupCheckResult,
    SourcePreviewResult, WorktreeFinishEffect, WorktreeFinishInternalMsg, WorktreeFinishMsg,
};
use gitcomet_core::auth::{AuthChallenge, AuthOutcome, GitAuthKind};
use gitcomet_core::domain::{CommitId, RepoSpec};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::services::{CancellationToken, CommandOutput, GitBackend};
use gitcomet_core::worktree_finish::{
    AgentDispatchAvailability, AgentDispatcher, AgentLaunch, AgentSession, AgentSessionState,
    FinishOperation, FinishSnapshot, FinishStepCommandOutput, IgnoredFilesFingerprint,
    InstalledAgent, IntegrationPreview, TreeId,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn cid(s: &str) -> CommitId {
    CommitId(s.into())
}

fn clean_snap(branch: &str, head: &str) -> FinishSnapshot {
    FinishSnapshot {
        worktree_exists: true,
        is_primary_worktree: false,
        is_bare: false,
        head: Some(cid(head)),
        head_parents: vec![cid("base000")],
        head_tree: Some(TreeId::from("tree_clean")),
        branch: Some(branch.to_string()),
        detached: false,
        operation: FinishOperation::None,
        has_merge_head: false,
        has_squash_msg: false,
        unmerged: Vec::new(),
        conflict_marker_paths: Vec::new(),
        staged_count: 0,
        unstaged_count: 0,
        untracked_count: 0,
        untracked_paths: Vec::new(),
        ignored_count: 0,
        ignored_sample: Vec::new(),
        ignored_fingerprint: Some(IgnoredFilesFingerprint::empty_verified()),
        dirty_entries: Some(Vec::new()),
        has_initialized_submodules: false,
        locked: false,
    }
}

fn clean_preview(ahead: usize, behind: usize, merged_tree: &str) -> IntegrationPreview {
    IntegrationPreview {
        ahead,
        behind,
        source_is_ancestor_of_target: ahead == 0,
        target_is_ancestor_of_source: behind == 0,
        merged_tree: Some(TreeId::from(merged_tree)),
        target_tree: Some(TreeId::from("tree_target")),
        conflict_paths: Vec::new(),
        linear: true,
        commit_subjects: vec!["feat commit".to_string()],
        likely_previously_squashed: false,
        touched_paths: vec![PathBuf::from("src/a.rs")],
    }
}

fn setup_state_with_repo(workdir: &Path) -> (AppState, RepositoryRuntimes, AtomicU64, RepoId) {
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: workdir.to_path_buf(),
        },
    );
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);
    (
        state,
        RepositoryRuntimes::default(),
        AtomicU64::new(10),
        repo_id,
    )
}

fn step_reduce(
    state: &mut AppState,
    repos: &mut RepositoryRuntimes,
    id_alloc: &AtomicU64,
    msg: Msg,
) -> Vec<Effect> {
    reducer::reduce(repos, id_alloc, state, msg)
}

fn preview_and_confirm_single(
    state: &mut AppState,
    repos: &mut RepositoryRuntimes,
    id_alloc: &AtomicU64,
    repo_id: RepoId,
    mode: FinishMode,
    squash: bool,
    remove_after: bool,
) -> (WorktreeFinishRunId, Vec<Effect>) {
    let main_path = PathBuf::from("/tmp/repo-main");
    let src_path = PathBuf::from("/tmp/repo-wt-a");
    let mut req = FinishSourceRequest::new(src_path.clone(), "feat/a".to_string());
    req.squash = squash;
    req.remove_after = remove_after;

    let effects = step_reduce(
        state,
        repos,
        id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
            repo_id,
            target_branch: "main".to_string(),
            mode,
            sources: vec![req],
            running_terminal_paths: Vec::new(),
        }),
    );
    assert_eq!(effects.len(), 1);
    let run_id = state.worktree_finish_runs[0].id;

    let mut target_snap = clean_snap("main", "target_0");
    target_snap.is_primary_worktree = true;
    let src_snap = clean_snap("feat/a", "src_a_1");
    let target_host = match mode {
        FinishMode::MergeInto => TargetHost::OpenTab {
            repo_id,
            path: main_path.clone(),
        },
        FinishMode::RebaseOnto => TargetHost::NotNeeded,
    };

    step_reduce(
        state,
        repos,
        id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::PreviewLoaded {
                run_id,
                start_index: 0,
                common_dir: PathBuf::from("/tmp/repo-main/.git"),
                target_head: Some(cid("target_0")),
                target_host,
                target_snapshot: (mode == FinishMode::MergeInto).then_some(target_snap),
                sources: vec![SourcePreviewResult {
                    path: src_path,
                    branch: "feat/a".to_string(),
                    locked_head: Some(cid("src_a_1")),
                    snapshot: Some(src_snap),
                    active_sessions: Vec::new(),
                    preview: Ok(clean_preview(1, 0, "tree_merged_a")),
                    default_squash_message: Some("Squash merge feat/a into main".to_string()),
                }],
                error: None,
            },
        ))),
    );

    let RunPhase::AwaitingConfirm { fingerprint } = state.worktree_finish_runs[0].phase else {
        panic!(
            "expected AwaitingConfirm, got {:?}",
            state.worktree_finish_runs[0].phase
        );
    };
    let confirm_effects = step_reduce(
        state,
        repos,
        id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
            run_id,
            fingerprint,
            running_terminal_paths: Vec::new(),
        }),
    );
    (run_id, confirm_effects)
}

#[test]
fn t_m1_m2_m3_m4_m5_m6_merge_outcomes_and_rerere_review() {
    // T-M1: Fast-forward
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        false,
        false,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Merge,
                before: clean_snap("main", "target_0"),
                after: Some(clean_snap("main", "src_a_1")),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git merge"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert_eq!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::Done(ItemOutcome::FastForwarded {
            new_head: cid("src_a_1")
        })
    );
    assert_eq!(state.worktree_finish_runs[0].phase, RunPhase::Finished);

    // T-M6: Merge conflict auto-staged by rerere must enter AwaitingReview, never auto-commit
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        false,
        false,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    let mut rerere_snap = clean_snap("main", "target_0");
    rerere_snap.operation = FinishOperation::Merge;
    rerere_snap.has_merge_head = true;
    rerere_snap.staged_count = 1;
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Merge,
                before: clean_snap("main", "target_0"),
                after: Some(rerere_snap),
                command: Ok(FinishStepCommandOutput {
                    output: CommandOutput {
                        exit_code: Some(1),
                        ..Default::default()
                    },
                    succeeded: false,
                }),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::AwaitingReview { .. }
    ));
}

#[test]
fn t_s1_s5_s6_squash_tree_verification_tampering_and_rerere_guard() {
    // T-S5: Clean squash where tree mismatches pre-step merge-tree MUST fail & pause
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        true,
        true,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: Some(TreeId::from("tree_expected")),
                error: None,
            },
        ))),
    );
    let mut staged_snap = clean_snap("main", "target_0");
    staged_snap.staged_count = 1;
    staged_snap.has_squash_msg = true;
    let commit_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::SquashMerge,
                before: clean_snap("main", "target_0"),
                after: Some(staged_snap),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git merge --squash"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert!(matches!(
        commit_effects.first(),
        Some(Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
            step: FinishStep::SquashCommit,
            ..
        }))
    ));

    // Tampered tree on resulting commit -> must fail!
    let mut tampered_after = clean_snap("main", "squashed_1");
    tampered_after.head_parents = vec![cid("target_0")];
    tampered_after.head_tree = Some(TreeId::from("tree_tampered"));
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::SquashCommit,
                before: clean_snap("main", "target_0"),
                after: Some(tampered_after),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git commit"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::Failed { .. }
    ));
    assert!(matches!(
        state.worktree_finish_runs[0].phase,
        RunPhase::Paused { current: 0 }
    ));

    // T-S6: Non-zero exit code with staged changes & no unmerged = rerere.autoUpdate -> AwaitingReview
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        true,
        false,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    let mut rerere_squash = clean_snap("main", "target_0");
    rerere_squash.staged_count = 1;
    rerere_squash.has_squash_msg = true;
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::SquashMerge,
                before: clean_snap("main", "target_0"),
                after: Some(rerere_squash),
                command: Ok(FinishStepCommandOutput {
                    output: CommandOutput {
                        exit_code: Some(1),
                        ..Default::default()
                    },
                    succeeded: false,
                }),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::AwaitingReview { .. }
    ));
}

#[test]
fn t_r2_r8_multi_stop_rebase_and_source_moved_disallows_keep_locked() {
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::RebaseOnto,
        false,
        false,
    );

    // T-R8: Source moved in Rebase mode -> KeepLockedHead must be ignored!
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: None,
                source_head: Some(cid("src_a_moved")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_moved")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::NeedsDecision {
            drift: DriftKind::SourceMoved { .. },
            ..
        }
    ));
    let keep_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::ResolveDrift {
            run_id,
            choice: DriftResolution::KeepLockedHead,
        }),
    );
    assert!(
        keep_effects.is_empty(),
        "RebaseOnto must NOT allow KeepLockedHead"
    );

    // F8: RefreshPreview must re-preview and require a fresh Confirm before preflighting.
    let refresh_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::ResolveDrift {
            run_id,
            choice: DriftResolution::RefreshPreview,
        }),
    );
    assert_eq!(state.worktree_finish_runs[0].phase, RunPhase::Previewing);
    assert!(matches!(
        refresh_effects.first(),
        Some(Effect::WorktreeFinish(WorktreeFinishEffect::LoadPreview {
            start_index: 0,
            ..
        }))
    ));
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::PreviewLoaded {
                run_id,
                start_index: 0,
                common_dir: PathBuf::from("/tmp/repo-main/.git"),
                target_head: Some(cid("target_0")),
                target_host: TargetHost::NotNeeded,
                target_snapshot: None,
                sources: vec![SourcePreviewResult {
                    path: PathBuf::from("/tmp/repo-wt-a"),
                    branch: "feat/a".to_string(),
                    locked_head: Some(cid("src_a_moved")),
                    snapshot: Some(clean_snap("feat/a", "src_a_moved")),
                    active_sessions: Vec::new(),
                    preview: Ok(clean_preview(2, 0, "tree_moved")),
                    default_squash_message: None,
                }],
                error: None,
            },
        ))),
    );
    let RunPhase::AwaitingConfirm {
        fingerprint: refreshed_fp,
    } = state.worktree_finish_runs[0].phase
    else {
        panic!("expected AwaitingConfirm after RefreshPreview");
    };
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
            run_id,
            fingerprint: refreshed_fp,
            running_terminal_paths: Vec::new(),
        }),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: None,
                source_head: Some(cid("src_a_moved")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_moved")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );

    // T-R2: Stop #1
    let mut rebase_conflict = clean_snap("feat/a", "src_a_moved");
    rebase_conflict.operation = FinishOperation::Rebase;
    rebase_conflict.unmerged = vec![PathBuf::from("file1.rs")];
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Rebase,
                before: clean_snap("feat/a", "src_a_moved"),
                after: Some(rebase_conflict.clone()),
                command: Ok(FinishStepCommandOutput {
                    output: CommandOutput {
                        exit_code: Some(1),
                        ..Default::default()
                    },
                    succeeded: false,
                }),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::PausedConflict { stops: 1, .. }
    ));

    // Resolve stop #1 -> Recheck -> AwaitingReview -> Continue -> Stop #2
    let mut resolved = rebase_conflict.clone();
    resolved.unmerged.clear();
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::RecheckLoaded {
                run_id,
                item: 0,
                snapshot: Ok(resolved),
                target_is_ancestor: Some(false),
                rebased_range: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::AwaitingReview { stops: 1, .. }
    ));
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Continue { run_id }),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::RebaseContinue,
                before: clean_snap("feat/a", "src_a_moved"),
                after: Some(rebase_conflict),
                command: Ok(FinishStepCommandOutput {
                    output: CommandOutput {
                        exit_code: Some(1),
                        ..Default::default()
                    },
                    succeeded: false,
                }),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::PausedConflict { stops: 2, .. }
    ));
}

#[test]
fn t_d2_d3_d4_a3_a4_snapshot_drift_and_agent_done_still_unmerged() {
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        false,
        true,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    let mut conflict_snap = clean_snap("main", "target_0");
    conflict_snap.operation = FinishOperation::Merge;
    conflict_snap.has_merge_head = true;
    conflict_snap.unmerged = vec![PathBuf::from("conflict.txt")];
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Merge,
                before: clean_snap("main", "target_0"),
                after: Some(conflict_snap.clone()),
                command: Ok(FinishStepCommandOutput {
                    output: CommandOutput {
                        exit_code: Some(1),
                        ..Default::default()
                    },
                    succeeded: false,
                }),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );

    // T-A2: Dispatch agent -> must target conflict site (/tmp/repo-main for Merge)
    let dispatch_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::DispatchAgent {
            run_id,
            agent_id: "claude".to_string(),
        }),
    );
    assert!(matches!(
        dispatch_effects.first(),
        Some(Effect::WorktreeFinish(WorktreeFinishEffect::LaunchAgent { site, .. }))
            if site == Path::new("/tmp/repo-main")
    ));
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::AgentLaunched {
                run_id,
                item: 0,
                agent_id: "claude".to_string(),
                request_id: 1,
                result: Ok(AgentLaunch {
                    session_id: Some("sess_1".to_string()),
                    operation_id: Some("op_1".to_string()),
                    outcome_kind: Some("launched".to_string()),
                    detail: None,
                }),
            },
        ))),
    );

    // T-A3: Agent reports `done`, triggers RecheckSite, but unmerged still present -> stays PausedConflict!
    let poll_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::AgentPolled {
                run_id,
                item: 0,
                sessions: Ok(vec![AgentSession {
                    session_id: "sess_1".to_string(),
                    agent_id: "claude".to_string(),
                    state: AgentSessionState::Done,
                    worktree_id: None,
                    updated_at: None,
                    last_assistant_message: Some("I think I fixed it".to_string()),
                }]),
            },
        ))),
    );
    assert!(matches!(
        poll_effects.first(),
        Some(Effect::WorktreeFinish(
            WorktreeFinishEffect::RecheckSite { .. }
        ))
    ));
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::RecheckLoaded {
                run_id,
                item: 0,
                snapshot: Ok(conflict_snap.clone()),
                target_is_ancestor: None,
                rebased_range: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::PausedConflict { .. }
    ));

    // T-D3: Externally aborted -> Aborted (never NothingNew!)
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::RecheckLoaded {
                run_id,
                item: 0,
                snapshot: Ok(clean_snap("main", "target_0")),
                target_is_ancestor: None,
                rebased_range: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::NeedsDecision {
            drift: DriftKind::Aborted { .. },
            ..
        }
    ));

    // T-D2 / T-A4: Externally committed merge -> AdoptCommit, adopting does NOT emit another commit!
    let mut adopted_snap = clean_snap("main", "merged_ext_1");
    adopted_snap.head_parents = vec![cid("target_0"), cid("src_a_1")];
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::RecheckLoaded {
                run_id,
                item: 0,
                snapshot: Ok(adopted_snap),
                target_is_ancestor: None,
                rebased_range: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::NeedsDecision {
            drift: DriftKind::AdoptCommit { .. },
            ..
        }
    ));
    let adopt_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::ResolveDrift {
            run_id,
            choice: DriftResolution::AdoptCommit,
        }),
    );
    // Must advance directly to CleanupCheck, NOT emit a RunStep commit!
    assert!(adopt_effects.iter().all(|e| !matches!(
        e,
        Effect::WorktreeFinish(WorktreeFinishEffect::RunStep { .. })
    )));
    assert_eq!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::Done(ItemOutcome::Merged {
            commit: cid("merged_ext_1")
        })
    );
}

#[test]
fn t_q7_q8_c2_c3_c5_c8_and_d4_cleanup_safety_guards() {
    // T-Q8: Confirm with wrong fingerprint rejects & re-previews
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let src_path = PathBuf::from("/tmp/repo-wt-a");
    let mut req = FinishSourceRequest::new(src_path.clone(), "feat/a".to_string());
    req.remove_after = true;
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
            repo_id,
            target_branch: "main".to_string(),
            mode: FinishMode::MergeInto,
            sources: vec![req.clone()],
            running_terminal_paths: Vec::new(),
        }),
    );
    let run_id = state.worktree_finish_runs[0].id;
    let mut target_snap = clean_snap("main", "target_0");
    target_snap.is_primary_worktree = true;
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::PreviewLoaded {
                run_id,
                start_index: 0,
                common_dir: PathBuf::from("/tmp/repo-main/.git"),
                target_head: Some(cid("target_0")),
                target_host: TargetHost::OpenTab {
                    repo_id,
                    path: PathBuf::from("/tmp/repo-main"),
                },
                target_snapshot: Some(target_snap),
                sources: vec![SourcePreviewResult {
                    path: src_path.clone(),
                    branch: "feat/a".to_string(),
                    locked_head: Some(cid("src_a_1")),
                    snapshot: Some(clean_snap("feat/a", "src_a_1")),
                    active_sessions: Vec::new(),
                    preview: Ok(clean_preview(1, 0, "tree_a")),
                    default_squash_message: None,
                }],
                error: None,
            },
        ))),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = state.worktree_finish_runs[0].phase else {
        panic!("expected AwaitingConfirm");
    };
    let bad_confirm = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
            run_id,
            fingerprint: fingerprint ^ 0xdead_beef,
            running_terminal_paths: Vec::new(),
        }),
    );
    assert!(matches!(
        bad_confirm.first(),
        Some(Effect::WorktreeFinish(
            WorktreeFinishEffect::LoadPreview { .. }
        ))
    ));
    assert_eq!(state.worktree_finish_runs[0].phase, RunPhase::Previewing);

    // Confirm properly, then verify T-Q7 rejects second run while running
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::PreviewLoaded {
                run_id,
                start_index: 0,
                common_dir: PathBuf::from("/tmp/repo-main/.git"),
                target_head: Some(cid("target_0")),
                target_host: TargetHost::OpenTab {
                    repo_id,
                    path: PathBuf::from("/tmp/repo-main"),
                },
                target_snapshot: Some(clean_snap("main", "target_0")),
                sources: vec![SourcePreviewResult {
                    path: src_path.clone(),
                    branch: "feat/a".to_string(),
                    locked_head: Some(cid("src_a_1")),
                    snapshot: Some(clean_snap("feat/a", "src_a_1")),
                    active_sessions: Vec::new(),
                    preview: Ok(clean_preview(1, 0, "tree_a")),
                    default_squash_message: None,
                }],
                error: None,
            },
        ))),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = state.worktree_finish_runs[0].phase else {
        panic!("expected AwaitingConfirm");
    };
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
            run_id,
            fingerprint,
            running_terminal_paths: vec![src_path.clone()], // T-C5: running terminal!
        }),
    );
    let second_run_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
            repo_id,
            target_branch: "main".to_string(),
            mode: FinishMode::MergeInto,
            sources: vec![req],
            running_terminal_paths: Vec::new(),
        }),
    );
    assert!(
        second_run_effects.is_empty(),
        "T-Q7: second concurrent run must be rejected"
    );

    // Finish item -> enters CleanupCheck -> T-C5 running GitComet terminal blocks removal!
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Merge,
                before: clean_snap("main", "target_0"),
                after: Some(clean_snap("main", "src_a_1")),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git merge"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert_eq!(state.worktree_finish_runs[0].phase, RunPhase::CleanupCheck);

    let cleanup_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::CleanupChecked {
                run_id,
                results: vec![SourceCleanupCheckResult {
                    index: 0,
                    snapshot: Ok(clean_snap("feat/a", "src_a_1")),
                    active_sessions: Err(AgentCheckStatus::OrcaNotFound),
                }],
                orca_unavailable: true,
                live_terminal_paths: Vec::new(),
            },
        ))),
    );
    assert!(cleanup_effects.iter().all(|e| !matches!(
        e,
        Effect::WorktreeFinish(WorktreeFinishEffect::RemoveWorktree { .. })
    )));
    assert_eq!(
        state.worktree_finish_runs[0].sources[0].cleanup,
        CleanupOutcome::Kept(CleanupBlocker::OpenGitCometTerminal)
    );
}

#[test]
fn t_auth_signing_passphrase_pauses_and_retries_without_duplicate_commit() {
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        true,
        false,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: Some(TreeId::from("tree_clean")),
                error: None,
            },
        ))),
    );
    let mut staged_snap = clean_snap("main", "target_0");
    staged_snap.staged_count = 1;
    staged_snap.has_squash_msg = true;
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::SquashMerge,
                before: clean_snap("main", "target_0"),
                after: Some(staged_snap.clone()),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git merge --squash"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );

    // SquashCommit hits SSH signing passphrase challenge
    let signing_err = Error::new(ErrorKind::Git(
        GitFailure::new(
            "git commit",
            GitFailureId::CommandFailed,
            Some(128),
            Vec::new(),
            b"Enter passphrase for key '/tmp/id_ed25519':".to_vec(),
            Some("Enter passphrase for key '/tmp/id_ed25519':".to_string()),
        )
        .with_auth_outcome(AuthOutcome::NeedsInput(AuthChallenge::from_prompt(
            GitAuthKind::Passphrase,
            "Enter passphrase for key '/tmp/id_ed25519':",
        ))),
    ));

    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::SquashCommit,
                before: staged_snap,
                after: None,
                command: Err(signing_err),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );

    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::AwaitingCredential {
            step: FinishStep::SquashCommit,
            ..
        }
    ));
    let prompt = state.auth_prompt.as_ref().expect("auth prompt installed");
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::WorktreeFinish { run_id }
    );

    // RetryCredential re-emits SquashCommit once
    let retry_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::RetryCredential { run_id }),
    );
    assert_eq!(retry_effects.len(), 1);
    assert!(matches!(
        retry_effects[0],
        Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
            step: FinishStep::SquashCommit,
            ..
        })
    ));
}

#[derive(Default)]
pub(super) struct FakeAgentDispatcher {
    availability: Mutex<Option<AgentDispatchAvailability>>,
    launched_workspaces: Mutex<Vec<PathBuf>>,
    sessions: Mutex<Vec<AgentSession>>,
}

impl FakeAgentDispatcher {
    pub(super) fn unavailable() -> Self {
        Self {
            availability: Mutex::new(Some(AgentDispatchAvailability::NotFound)),
            launched_workspaces: Mutex::new(Vec::new()),
            sessions: Mutex::new(Vec::new()),
        }
    }
}

impl AgentDispatcher for FakeAgentDispatcher {
    fn availability(&self) -> AgentDispatchAvailability {
        self.availability
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(AgentDispatchAvailability::NotFound)
    }

    fn installed(
        &self,
        _workspace: &Path,
        _cancellation: &CancellationToken,
    ) -> std::result::Result<Vec<InstalledAgent>, Error> {
        Ok(vec![InstalledAgent {
            id: "claude".to_string(),
            name: "Claude".to_string(),
            is_default: true,
        }])
    }

    fn launch(
        &self,
        workspace: &Path,
        _agent_id: &str,
        _prompt: &str,
        _retry_operation_id: Option<&str>,
    ) -> std::result::Result<AgentLaunch, Error> {
        self.launched_workspaces
            .lock()
            .unwrap()
            .push(workspace.to_path_buf());
        Ok(AgentLaunch {
            session_id: Some("fake_sess_1".to_string()),
            operation_id: Some("fake_op_1".to_string()),
            outcome_kind: Some("launched".to_string()),
            detail: None,
        })
    }

    fn sessions(
        &self,
        _workspace: &Path,
        _cancellation: &CancellationToken,
    ) -> std::result::Result<Vec<AgentSession>, Error> {
        Ok(self.sessions.lock().unwrap().clone())
    }
}

#[test]
fn end_to_end_store_batch_merge_squash_and_safe_cleanup_on_real_temp_repo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = dir.path().join("main");
    let wt_a = dir.path().join("wt-a");
    let wt_b = dir.path().join("wt-b");
    std::fs::create_dir_all(&main_repo).expect("mkdir main");

    run_git(&main_repo, &["init", "-b", "main"]);
    run_git(&main_repo, &["config", "user.email", "test@example.com"]);
    run_git(&main_repo, &["config", "user.name", "Tester"]);
    run_git(&main_repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(main_repo.join("seed.txt"), "seed\n").expect("seed");
    run_git(&main_repo, &["add", "seed.txt"]);
    run_git(&main_repo, &["commit", "-m", "initial"]);

    run_git(
        &main_repo,
        &["worktree", "add", "-b", "feat/a", wt_a.to_str().unwrap()],
    );
    run_git(
        &main_repo,
        &["worktree", "add", "-b", "feat/b", wt_b.to_str().unwrap()],
    );

    std::fs::write(wt_a.join("a.txt"), "from a\n").expect("write a");
    run_git(&wt_a, &["add", "a.txt"]);
    run_git(&wt_a, &["commit", "-m", "add a"]);

    std::fs::write(wt_b.join("b1.txt"), "b1\n").expect("write b1");
    run_git(&wt_b, &["add", "b1.txt"]);
    run_git(&wt_b, &["commit", "-m", "add b1"]);
    std::fs::write(wt_b.join("b2.txt"), "b2\n").expect("write b2");
    run_git(&wt_b, &["add", "b2.txt"]);
    run_git(&wt_b, &["commit", "-m", "add b2"]);

    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let dispatcher = Arc::new(FakeAgentDispatcher::unavailable());
    let (store, _rx) = AppStore::new_test_with_agent_dispatcher(backend, dispatcher);
    store.dispatch(Msg::OpenRepo(main_repo.clone()));

    let deadline = Instant::now() + Duration::from_secs(10);
    let repo_id = loop {
        let snap = store.snapshot();
        if let Some(repo) = snap.repos.first()
            && matches!(repo.open, Loadable::Ready(()))
        {
            break repo.id;
        }
        assert!(Instant::now() < deadline, "timed out opening temp repo");
        std::thread::sleep(Duration::from_millis(20));
    };

    let mut req_a = FinishSourceRequest::new(wt_a.clone(), "feat/a".to_string());
    req_a.remove_after = true;
    let mut req_b = FinishSourceRequest::new(wt_b.clone(), "feat/b".to_string());
    req_b.squash = true;
    req_b.remove_after = true;

    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
        repo_id,
        target_branch: "main".to_string(),
        mode: FinishMode::MergeInto,
        sources: vec![req_a, req_b],
        running_terminal_paths: Vec::new(),
    }));

    let (run_id, fingerprint) = loop {
        let snap = store.snapshot();
        if let Some(run) = snap.worktree_finish_runs.first()
            && let RunPhase::AwaitingConfirm { fingerprint } = run.phase
        {
            assert!(
                run.can_confirm(),
                "expected confirmable preview, blockers={:?}",
                run.preview_blockers
            );
            break (run.id, fingerprint);
        }
        assert!(Instant::now() < deadline, "timed out waiting for preview");
        std::thread::sleep(Duration::from_millis(20));
    };

    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));

    loop {
        let snap = store.snapshot();
        if let Some(run) = snap.worktree_finish_runs.iter().find(|r| r.id == run_id)
            && run.phase == RunPhase::Finished
        {
            assert!(matches!(
                run.sources[0].phase,
                ItemPhase::Done(ItemOutcome::FastForwarded { .. })
            ));
            assert!(matches!(
                run.sources[1].phase,
                ItemPhase::Done(ItemOutcome::Squashed {
                    content_verified: SquashContentCheck::Verified,
                    ..
                })
            ));
            assert_eq!(run.sources[0].cleanup, CleanupOutcome::Removed);
            assert_eq!(run.sources[1].cleanup, CleanupOutcome::Removed);
            assert!(run.cleanup_agent_check_unverified);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for batch finish; state={:?}",
            store.snapshot().worktree_finish_runs
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // Both worktree folders removed, both files landed on main, AND both local branches preserved!
    assert!(!wt_a.exists());
    assert!(!wt_b.exists());
    assert!(main_repo.join("a.txt").is_file());
    assert!(main_repo.join("b1.txt").is_file());
    assert!(main_repo.join("b2.txt").is_file());
    let branches = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(&main_repo)
        .args(["branch", "--list", "feat/*"])
        .output()
        .expect("git branch");
    let branches_text = String::from_utf8_lossy(&branches.stdout);
    assert!(
        branches_text.contains("feat/a"),
        "feat/a branch must be preserved"
    );
    assert!(
        branches_text.contains("feat/b"),
        "feat/b branch must be preserved"
    );
}

#[test]
fn t_q1_q2_q3_multi_item_queue_skip_and_abort_remaining() {
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let src_a = PathBuf::from("/tmp/repo-wt-a");
    let src_b = PathBuf::from("/tmp/repo-wt-b");
    let src_c = PathBuf::from("/tmp/repo-wt-c");
    let mut req_a = FinishSourceRequest::new(src_a.clone(), "feat/a".to_string());
    req_a.remove_after = true;
    let mut req_b = FinishSourceRequest::new(src_b.clone(), "feat/b".to_string());
    req_b.remove_after = true;
    let mut req_c = FinishSourceRequest::new(src_c.clone(), "feat/c".to_string());
    req_c.remove_after = true;

    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
            repo_id,
            target_branch: "main".to_string(),
            mode: FinishMode::MergeInto,
            sources: vec![req_a, req_b, req_c],
            running_terminal_paths: Vec::new(),
        }),
    );
    let run_id = state.worktree_finish_runs[0].id;
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::PreviewLoaded {
                run_id,
                start_index: 0,
                common_dir: PathBuf::from("/tmp/repo-main/.git"),
                target_head: Some(cid("target_0")),
                target_host: TargetHost::OpenTab {
                    repo_id,
                    path: PathBuf::from("/tmp/repo-main"),
                },
                target_snapshot: Some(clean_snap("main", "target_0")),
                sources: vec![
                    SourcePreviewResult {
                        path: src_a.clone(),
                        branch: "feat/a".to_string(),
                        locked_head: Some(cid("src_a_1")),
                        snapshot: Some(clean_snap("feat/a", "src_a_1")),
                        active_sessions: Vec::new(),
                        preview: Ok(clean_preview(1, 0, "tree_a")),
                        default_squash_message: None,
                    },
                    SourcePreviewResult {
                        path: src_b.clone(),
                        branch: "feat/b".to_string(),
                        locked_head: Some(cid("src_b_1")),
                        snapshot: Some(clean_snap("feat/b", "src_b_1")),
                        active_sessions: Vec::new(),
                        preview: Ok(clean_preview(1, 1, "tree_b")),
                        default_squash_message: None,
                    },
                    SourcePreviewResult {
                        path: src_c.clone(),
                        branch: "feat/c".to_string(),
                        locked_head: Some(cid("src_c_1")),
                        snapshot: Some(clean_snap("feat/c", "src_c_1")),
                        active_sessions: Vec::new(),
                        preview: Ok(clean_preview(1, 1, "tree_c")),
                        default_squash_message: None,
                    },
                ],
                error: None,
            },
        ))),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = state.worktree_finish_runs[0].phase else {
        panic!("expected AwaitingConfirm");
    };
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
            run_id,
            fingerprint,
            running_terminal_paths: Vec::new(),
        }),
    );

    // Item A succeeds
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Merge,
                before: clean_snap("main", "target_0"),
                after: Some(clean_snap("main", "src_a_1")),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git merge"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );

    // Item B hits conflict -> T-Q1: A Done, B PausedConflict, C Queued
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 1,
                target_head: Some(cid("src_a_1")),
                target_snapshot: Some(clean_snap("main", "src_a_1")),
                source_head: Some(cid("src_b_1")),
                source_snapshot: Some(clean_snap("feat/b", "src_b_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    let mut conflict_b = clean_snap("main", "src_a_1");
    conflict_b.operation = FinishOperation::Merge;
    conflict_b.has_merge_head = true;
    conflict_b.unmerged = vec![PathBuf::from("b.rs")];
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 1,
                step: FinishStep::Merge,
                before: clean_snap("main", "src_a_1"),
                after: Some(conflict_b),
                command: Ok(FinishStepCommandOutput {
                    output: CommandOutput {
                        exit_code: Some(1),
                        ..Default::default()
                    },
                    succeeded: false,
                }),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::Done(_)
    ));
    assert!(matches!(
        state.worktree_finish_runs[0].sources[1].phase,
        ItemPhase::PausedConflict { .. }
    ));
    assert_eq!(
        state.worktree_finish_runs[0].sources[2].phase,
        ItemPhase::Queued
    );

    // T-Q3: AbortRemaining(keep_conflict_state: true) skips B & C and runs CleanupCheck ONLY for A!
    let abort_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::AbortRemaining {
            run_id,
            keep_conflict_state: true,
        }),
    );
    assert_eq!(state.worktree_finish_runs[0].phase, RunPhase::CleanupCheck);
    assert!(matches!(
        abort_effects.first(),
        Some(Effect::WorktreeFinish(WorktreeFinishEffect::CheckCleanup { items, .. }))
            if items.len() == 1 && items[0].0 == 0
    ));
    // T-C7: skipped items B and C must not be cleaned
    assert_eq!(
        state.worktree_finish_runs[0].sources[1].cleanup,
        CleanupOutcome::NotRequested
    );
    assert_eq!(
        state.worktree_finish_runs[0].sources[2].cleanup,
        CleanupOutcome::NotRequested
    );
}

#[test]
fn t_x3_d1_unchecked_out_target_creates_and_cleans_temp_worktree_on_real_repo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let main_repo = dir.path().join("main");
    let wt_feat = dir.path().join("wt-feat");
    std::fs::create_dir_all(&main_repo).expect("mkdir main");

    run_git(&main_repo, &["init", "-b", "main"]);
    run_git(&main_repo, &["config", "user.email", "test@example.com"]);
    run_git(&main_repo, &["config", "user.name", "Tester"]);
    run_git(&main_repo, &["config", "commit.gpgsign", "false"]);
    std::fs::write(main_repo.join("seed.txt"), "seed\n").expect("seed");
    run_git(&main_repo, &["add", "seed.txt"]);
    run_git(&main_repo, &["commit", "-m", "initial"]);

    // Create local branch `release` NOT checked out in any worktree!
    run_git(&main_repo, &["branch", "release"]);
    run_git(
        &main_repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/rel",
            wt_feat.to_str().unwrap(),
        ],
    );
    std::fs::write(wt_feat.join("rel.txt"), "release feature\n").expect("write rel");
    run_git(&wt_feat, &["add", "rel.txt"]);
    run_git(&wt_feat, &["commit", "-m", "add rel"]);

    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let dispatcher = Arc::new(FakeAgentDispatcher::unavailable());
    let (store, _rx) = AppStore::new_test_with_agent_dispatcher(backend, dispatcher);
    store.dispatch(Msg::OpenRepo(main_repo.clone()));

    let deadline = Instant::now() + Duration::from_secs(10);
    let repo_id = loop {
        let snap = store.snapshot();
        if let Some(repo) = snap.repos.first()
            && matches!(repo.open, Loadable::Ready(()))
        {
            break repo.id;
        }
        assert!(Instant::now() < deadline, "timed out opening repo");
        std::thread::sleep(Duration::from_millis(20));
    };

    let req = FinishSourceRequest::new(wt_feat.clone(), "feat/rel".to_string());
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
        repo_id,
        target_branch: "release".to_string(),
        mode: FinishMode::MergeInto,
        sources: vec![req],
        running_terminal_paths: Vec::new(),
    }));

    let (run_id, fingerprint, temp_path) = loop {
        let snap = store.snapshot();
        if let Some(run) = snap.worktree_finish_runs.first()
            && let RunPhase::AwaitingConfirm { fingerprint } = run.phase
        {
            let TargetHost::Temporary {
                path,
                created: false,
            } = &run.target.host
            else {
                panic!(
                    "D1: expected uncreated Temporary host before confirm, got {:?}",
                    run.target.host
                );
            };
            assert!(
                !path.exists(),
                "D1: temporary worktree must NOT exist before confirmation"
            );
            break (run.id, fingerprint, path.clone());
        }
        assert!(Instant::now() < deadline, "timed out waiting for preview");
        std::thread::sleep(Duration::from_millis(20));
    };

    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));

    loop {
        let snap = store.snapshot();
        if let Some(run) = snap.worktree_finish_runs.iter().find(|r| r.id == run_id)
            && run.phase == RunPhase::Finished
        {
            assert!(matches!(
                run.sources[0].phase,
                ItemPhase::Done(ItemOutcome::FastForwarded { .. })
            ));
            assert!(run.temp_worktree_kept_note.is_none());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for temporary worktree finish; runs={:?}",
            store.snapshot().worktree_finish_runs
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    assert!(
        !temp_path.exists(),
        "D1: clean temporary worktree must be removed after run finishes"
    );
    let release_sha = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(&main_repo)
        .args(["rev-parse", "refs/heads/release"])
        .output()
        .expect("rev-parse release");
    let feat_sha = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(&main_repo)
        .args(["rev-parse", "refs/heads/feat/rel"])
        .output()
        .expect("rev-parse feat/rel");
    assert_eq!(release_sha.stdout, feat_sha.stdout);
}

#[test]
fn t_d3_ignored_files_change_blocks_cleanup_and_d4_dispatched_unverifiable_blocks_cleanup() {
    // D3: if ignored_count changed between preview and cleanup check -> Kept(IgnoredFilesChangedOrUnverified)
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        false,
        true,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Merge,
                before: clean_snap("main", "target_0"),
                after: Some(clean_snap("main", "src_a_1")),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git merge"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );

    let mut changed_ignored = clean_snap("feat/a", "src_a_1");
    changed_ignored.ignored_count = 3; // Was 0 at preview!
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::CleanupChecked {
                run_id,
                results: vec![SourceCleanupCheckResult {
                    index: 0,
                    snapshot: Ok(changed_ignored),
                    active_sessions: Err(AgentCheckStatus::OrcaNotFound),
                }],
                orca_unavailable: true,
                live_terminal_paths: Vec::new(),
            },
        ))),
    );
    assert_eq!(
        state.worktree_finish_runs[0].sources[0].cleanup,
        CleanupOutcome::Kept(CleanupBlocker::IgnoredFilesChangedOrUnverified)
    );

    // D4a: Active Orca agent -> Kept(ActiveOrcaAgent)
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        false,
        true,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Merge,
                before: clean_snap("main", "target_0"),
                after: Some(clean_snap("main", "src_a_1")),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git merge"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::CleanupChecked {
                run_id,
                results: vec![SourceCleanupCheckResult {
                    index: 0,
                    snapshot: Ok(clean_snap("feat/a", "src_a_1")),
                    active_sessions: Ok(vec![AgentSession {
                        session_id: "sess_live".to_string(),
                        agent_id: "claude".to_string(),
                        state: AgentSessionState::Working,
                        worktree_id: None,
                        updated_at: None,
                        last_assistant_message: None,
                    }]),
                }],
                orca_unavailable: false,
                live_terminal_paths: Vec::new(),
            },
        ))),
    );
    assert_eq!(
        state.worktree_finish_runs[0].sources[0].cleanup,
        CleanupOutcome::Kept(CleanupBlocker::ActiveOrcaAgent)
    );

    // D4b: Dispatched agent + Orca unavailable at cleanup -> Kept(OrcaSessionUnverifiable)
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        false,
        true,
    );
    state.worktree_finish_runs[0].sources[0].dispatched_agent = true;
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: Some(clean_snap("main", "target_0")),
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Merge,
                before: clean_snap("main", "target_0"),
                after: Some(clean_snap("main", "src_a_1")),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git merge"),
                )),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::CleanupChecked {
                run_id,
                results: vec![SourceCleanupCheckResult {
                    index: 0,
                    snapshot: Ok(clean_snap("feat/a", "src_a_1")),
                    active_sessions: Err(AgentCheckStatus::OrcaNotFound),
                }],
                orca_unavailable: true,
                live_terminal_paths: Vec::new(),
            },
        ))),
    );
    assert_eq!(
        state.worktree_finish_runs[0].sources[0].cleanup,
        CleanupOutcome::Kept(CleanupBlocker::OrcaSessionUnverifiable)
    );
}

pub(super) fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let out = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub(super) fn setup_real_finish_fixture(conflict: bool) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let main = dir.path().join("main");
    let src = dir.path().join("source");
    std::fs::create_dir_all(&main).expect("mkdir main");

    run_git(&main, &["init", "-b", "main"]);
    run_git(&main, &["config", "user.email", "test@example.com"]);
    run_git(&main, &["config", "user.name", "Tester"]);
    run_git(&main, &["config", "commit.gpgsign", "false"]);
    std::fs::write(main.join("base.txt"), "base\n").expect("write base");
    std::fs::write(main.join(".gitignore"), "*.ignored\n").expect("write gitignore");
    run_git(&main, &["add", "base.txt", ".gitignore"]);
    run_git(&main, &["commit", "-m", "initial"]);

    run_git(
        &main,
        &["worktree", "add", "-b", "feat", src.to_str().unwrap()],
    );
    if conflict {
        std::fs::write(main.join("base.txt"), "target\n").expect("main conflict");
        run_git(&main, &["add", "base.txt"]);
        run_git(&main, &["commit", "-m", "target edits base"]);
        std::fs::write(src.join("base.txt"), "source\n").expect("src conflict");
        run_git(&src, &["add", "base.txt"]);
        run_git(&src, &["commit", "-m", "source edits base"]);
    } else {
        std::fs::write(main.join("target.txt"), "target\n").expect("target file");
        run_git(&main, &["add", "target.txt"]);
        run_git(&main, &["commit", "-m", "target commit"]);
        std::fs::write(src.join("feature.txt"), "feature\n").expect("feature file");
        run_git(&src, &["add", "feature.txt"]);
        run_git(&src, &["commit", "-m", "feat 1"]);
        std::fs::write(src.join("more.txt"), "more\n").expect("more file");
        run_git(&src, &["add", "more.txt"]);
        run_git(&src, &["commit", "-m", "feat 2"]);
    }
    (dir, main, src)
}

pub(super) fn wait_real_run(
    store: &AppStore,
    predicate: impl Fn(&WorktreeFinishRun) -> bool,
) -> WorktreeFinishRun {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(run) = store.snapshot().worktree_finish_runs.last()
            && predicate(run)
        {
            return run.clone();
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for worktree finish run; state={:?}",
            store.snapshot().worktree_finish_runs
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

pub(super) fn open_real_store_and_preview(
    main: &Path,
    src: &Path,
    mode: FinishMode,
    squash: bool,
    remove_after: bool,
    dispatcher: Arc<dyn AgentDispatcher>,
) -> (
    AppStore,
    smol::channel::Receiver<StoreEvent>,
    WorktreeFinishRun,
) {
    open_real_store_and_preview_onto(main, src, "main", mode, squash, remove_after, dispatcher)
}

fn open_real_store_and_preview_onto(
    main: &Path,
    src: &Path,
    target_branch: &str,
    mode: FinishMode,
    squash: bool,
    remove_after: bool,
    dispatcher: Arc<dyn AgentDispatcher>,
) -> (
    AppStore,
    smol::channel::Receiver<StoreEvent>,
    WorktreeFinishRun,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (store, rx) = AppStore::new_test_with_agent_dispatcher(backend, dispatcher);
    store.dispatch(Msg::OpenRepo(main.to_path_buf()));
    let deadline = Instant::now() + Duration::from_secs(10);
    let repo_id = loop {
        let snap = store.snapshot();
        if let Some(repo) = snap.repos.first()
            && matches!(repo.open, Loadable::Ready(()))
        {
            break repo.id;
        }
        assert!(Instant::now() < deadline, "timed out opening repo");
        std::thread::sleep(Duration::from_millis(15));
    };
    let mut req = FinishSourceRequest::new(src.to_path_buf(), "feat".to_string());
    assert!(!req.squash, "F7: squash must default to false");
    assert!(!req.remove_after, "F7: remove_after must default to false");
    req.squash = squash;
    req.remove_after = remove_after;
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Preview {
        repo_id,
        target_branch: target_branch.to_string(),
        mode,
        sources: vec![req],
        running_terminal_paths: Vec::new(),
    }));
    let run = wait_real_run(&store, |r| {
        matches!(r.phase, RunPhase::AwaitingConfirm { .. })
    });
    assert!(
        run.can_confirm(),
        "preview blockers: {:?}",
        run.preview_blockers
    );
    (store, rx, run)
}

#[test]
fn f1_same_sha_branch_switch_refuses_merge_and_rebase_without_mutating_wrong_branch() {
    // MergeInto: switch target worktree to `other` at the exact same SHA before Confirm.
    let (_dir, main, src) = setup_real_finish_fixture(false);
    let main_before = git_stdout(&main, &["rev-parse", "refs/heads/main"]);
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    run_git(&main, &["checkout", "-b", "other"]);
    let other_before = git_stdout(&main, &["rev-parse", "refs/heads/other"]);
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    let settled = wait_real_run(&store, |r| {
        matches!(
            r.phase,
            RunPhase::Paused { .. } | RunPhase::Finished | RunPhase::Aborted
        )
    });
    assert!(
        matches!(
            settled.sources[0].phase,
            ItemPhase::NeedsDecision {
                drift: DriftKind::Unexpected { .. },
                ..
            }
        ),
        "F1: expected refusal on switched target branch, got {:?}",
        settled.sources[0].phase
    );
    assert_eq!(
        git_stdout(&main, &["rev-parse", "refs/heads/main"]),
        main_before
    );
    assert_eq!(
        git_stdout(&main, &["rev-parse", "refs/heads/other"]),
        other_before
    );
    assert!(
        !main.join("feature.txt").exists(),
        "other branch must NOT receive feature.txt"
    );

    // RebaseOnto: switch source worktree to `other` at the exact same SHA before Confirm.
    let (_dir2, main2, src2) = setup_real_finish_fixture(false);
    let feat_before = git_stdout(&main2, &["rev-parse", "refs/heads/feat"]);
    let (store2, _rx2, run2) = open_real_store_and_preview(
        &main2,
        &src2,
        FinishMode::RebaseOnto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    run_git(&src2, &["checkout", "-b", "other"]);
    let other2_before = git_stdout(&src2, &["rev-parse", "refs/heads/other"]);
    let RunPhase::AwaitingConfirm { fingerprint: fp2 } = run2.phase else {
        panic!("expected AwaitingConfirm");
    };
    store2.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run2.id,
        fingerprint: fp2,
        running_terminal_paths: Vec::new(),
    }));
    let settled2 = wait_real_run(&store2, |r| {
        matches!(
            r.phase,
            RunPhase::Paused { .. } | RunPhase::Finished | RunPhase::Aborted
        )
    });
    assert!(
        matches!(
            settled2.sources[0].phase,
            ItemPhase::NeedsDecision {
                drift: DriftKind::Unexpected { .. },
                ..
            }
        ),
        "F1: expected refusal on switched source branch, got {:?}",
        settled2.sources[0].phase
    );
    assert_eq!(
        git_stdout(&main2, &["rev-parse", "refs/heads/feat"]),
        feat_before
    );
    assert_eq!(
        git_stdout(&src2, &["rev-parse", "refs/heads/other"]),
        other2_before
    );
    assert!(
        !src2.join("target.txt").exists(),
        "other branch must NOT be rebased onto main"
    );
}

#[test]
fn f2_ignored_file_modified_with_same_count_preserves_source_worktree_and_data() {
    let (_dir, main, src) = setup_real_finish_fixture(false);
    std::fs::write(src.join("local.ignored"), "preview data").expect("write initial ignored");
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        false,
        true,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    // Modify same ignored file (ignored_count stays 1!)
    std::fs::write(src.join("local.ignored"), "NEW USER DATA AFTER PREVIEW")
        .expect("overwrite ignored");
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    let done = wait_real_run(&store, |r| !r.is_active());
    assert!(matches!(
        done.sources[0].phase,
        ItemPhase::Done(ItemOutcome::Merged { .. })
    ));
    assert_eq!(
        done.sources[0].cleanup,
        CleanupOutcome::Kept(CleanupBlocker::IgnoredFilesChangedOrUnverified)
    );
    assert!(
        src.exists(),
        "source worktree must be preserved when ignored content changed"
    );
    assert_eq!(
        std::fs::read_to_string(src.join("local.ignored")).expect("read preserved ignored"),
        "NEW USER DATA AFTER PREVIEW"
    );
    assert!(!git_stdout(&main, &["rev-parse", "refs/heads/feat"]).is_empty());

    // Conversely, if ignored file was disclosed and UNCHANGED, non-force removal succeeds and keeps branch.
    let (_dir2, main2, src2) = setup_real_finish_fixture(false);
    std::fs::write(src2.join("local.ignored"), "unchanged build artifact").expect("write ignored");
    let (store2, _rx2, run2) = open_real_store_and_preview(
        &main2,
        &src2,
        FinishMode::MergeInto,
        false,
        true,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint: fp2 } = run2.phase else {
        panic!("expected AwaitingConfirm");
    };
    store2.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run2.id,
        fingerprint: fp2,
        running_terminal_paths: Vec::new(),
    }));
    let done2 = wait_real_run(&store2, |r| !r.is_active());
    assert_eq!(done2.sources[0].cleanup, CleanupOutcome::Removed);
    assert!(!src2.exists());
    assert!(!git_stdout(&main2, &["rev-parse", "refs/heads/feat"]).is_empty());
}

#[test]
fn f3_squash_merge_git_refusal_never_reports_nothing_new_or_removes_source() {
    let (_dir, main, src) = setup_real_finish_fixture(false);
    let main_before = git_stdout(&main, &["rev-parse", "refs/heads/main"]);
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        true,
        true,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    // Add colliding untracked file after preview so git merge --squash refuses to overwrite it.
    std::fs::write(main.join("feature.txt"), "untracked user content").expect("write untracked");
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    let settled = wait_real_run(&store, |r| {
        matches!(
            r.phase,
            RunPhase::Paused { .. } | RunPhase::Finished | RunPhase::Aborted
        )
    });
    assert!(
        matches!(settled.sources[0].phase, ItemPhase::Failed { .. }),
        "F3: git refusal must fail & pause, got {:?}",
        settled.sources[0].phase
    );
    assert_eq!(settled.phase, RunPhase::Paused { current: 0 });
    assert_eq!(
        git_stdout(&main, &["rev-parse", "refs/heads/main"]),
        main_before
    );
    assert!(
        src.exists(),
        "F3: source worktree must NOT be deleted on squash failure"
    );
    assert_eq!(
        std::fs::read_to_string(main.join("feature.txt")).expect("read untracked"),
        "untracked user content"
    );
}

#[test]
fn f4_skip_conflict_preserves_post_conflict_staged_and_edited_user_data() {
    // Case A: Squash conflict + unrelated staged file -> scoped abort cleans conflict AND preserves new file!
    let (_dir, main, src) = setup_real_finish_fixture(true);
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        true,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    let paused = wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    assert_eq!(paused.phase, RunPhase::Paused { current: 0 });

    std::fs::write(main.join("new-user-data.txt"), "NEW DATA AFTER CONFLICT")
        .expect("write new user data");
    run_git(&main, &["add", "new-user-data.txt"]);

    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem {
        run_id: run.id,
    }));
    let done = wait_real_run(&store, |r| !r.is_active());
    assert!(matches!(
        done.sources[0].phase,
        ItemPhase::Skipped(SkipReason::UserSkipped)
    ));
    assert_eq!(
        std::fs::read_to_string(main.join("new-user-data.txt")).expect("user file preserved"),
        "NEW DATA AFTER CONFLICT"
    );
    let status = git_stdout(&main, &["status", "--porcelain"]);
    assert!(
        status.contains("A  new-user-data.txt"),
        "F4: unrelated staged user file must remain staged; status:\n{status}"
    );
    assert_eq!(
        std::fs::read_to_string(main.join("base.txt")).expect("base restored"),
        "target\n"
    );

    // Case B: Squash conflict + user edited conflicted file -> abort refused, user edits preserved!
    let (_dir2, main2, src2) = setup_real_finish_fixture(true);
    let (store2, _rx2, run2) = open_real_store_and_preview(
        &main2,
        &src2,
        FinishMode::MergeInto,
        true,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint: fp2 } = run2.phase else {
        panic!("expected AwaitingConfirm");
    };
    store2.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run2.id,
        fingerprint: fp2,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store2, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    std::fs::write(main2.join("base.txt"), "user custom resolution draft\n").expect("edit base");
    store2.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem {
        run_id: run2.id,
    }));
    let failed = wait_real_run(&store2, |r| {
        matches!(r.sources[0].phase, ItemPhase::Failed { .. })
    });
    assert_eq!(failed.phase, RunPhase::Paused { current: 0 });
    assert_eq!(
        std::fs::read_to_string(main2.join("base.txt")).expect("user edits preserved"),
        "user custom resolution draft\n"
    );

    // Case C: Normal Merge conflict + unrelated staged user file -> destructive `merge --abort` refused!
    let (_dir3, main3, src3) = setup_real_finish_fixture(true);
    let (store3, _rx3, run3) = open_real_store_and_preview(
        &main3,
        &src3,
        FinishMode::MergeInto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint: fp3 } = run3.phase else {
        panic!("expected AwaitingConfirm");
    };
    store3.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run3.id,
        fingerprint: fp3,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store3, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    std::fs::write(main3.join("new-user-data.txt"), "MERGE CONFLICT USER DATA")
        .expect("write user data");
    run_git(&main3, &["add", "new-user-data.txt"]);
    store3.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem {
        run_id: run3.id,
    }));
    let failed3 = wait_real_run(&store3, |r| {
        matches!(r.sources[0].phase, ItemPhase::Failed { .. })
    });
    assert_eq!(failed3.phase, RunPhase::Paused { current: 0 });
    assert_eq!(
        std::fs::read_to_string(main3.join("new-user-data.txt")).expect("preserved"),
        "MERGE CONFLICT USER DATA"
    );
    assert!(
        git_stdout(&main3, &["status", "--porcelain"]).contains("A  new-user-data.txt"),
        "F4: staged file must remain intact when merge abort is refused"
    );

    // Case D: Rebase conflict + post-conflict staged user file -> AbortRemaining(keep=false) refuses destructive rebase --abort!
    let (_dir4, main4, src4) = setup_real_finish_fixture(true);
    let main4_before = git_stdout(&main4, &["rev-parse", "refs/heads/main"]);
    let (store4, _rx4, run4) = open_real_store_and_preview(
        &main4,
        &src4,
        FinishMode::RebaseOnto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint: fp4 } = run4.phase else {
        panic!("expected AwaitingConfirm");
    };
    store4.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run4.id,
        fingerprint: fp4,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store4, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    std::fs::write(
        src4.join("rebase-user-data.txt"),
        "REBASE CONFLICT USER DATA",
    )
    .expect("write rebase user data");
    run_git(&src4, &["add", "rebase-user-data.txt"]);
    store4.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::AbortRemaining {
        run_id: run4.id,
        keep_conflict_state: false,
    }));
    let failed4 = wait_real_run(&store4, |r| {
        matches!(r.sources[0].phase, ItemPhase::Failed { .. })
    });
    assert_eq!(failed4.phase, RunPhase::Paused { current: 0 });
    assert_eq!(
        std::fs::read_to_string(src4.join("rebase-user-data.txt")).expect("preserved"),
        "REBASE CONFLICT USER DATA"
    );
    assert!(
        git_stdout(&src4, &["status", "--porcelain"]).contains("A  rebase-user-data.txt"),
        "F4: staged file in source worktree must remain intact when rebase abort is refused"
    );
    assert_eq!(
        git_stdout(&main4, &["rev-parse", "refs/heads/main"]),
        main4_before
    );
}

#[test]
fn f4_skip_squash_conflict_with_star_and_magic_filenames_preserves_unrelated_staged_data() {
    #[cfg(unix)]
    let magic_cases: &[&str] = &[
        "*",
        ":(glob)*",
        "?",
        "[a]",
        ":(icase)user.txt",
        ":!user.txt",
    ];
    #[cfg(not(unix))]
    let magic_cases: &[&str] = &["[a]"];

    for magic in magic_cases {
        let (_dir, main, src) = setup_real_finish_fixture(true);
        for path in [&main, &src] {
            std::fs::rename(path.join("base.txt"), path.join(magic))
                .expect("rename conflict file to magic name");
            run_git(path, &["--literal-pathspecs", "add", "-A"]);
            run_git(path, &["commit", "-m", "rename conflict file"]);
        }
        std::fs::write(main.join("user.txt"), "original user content\n").expect("write user.txt");
        run_git(&main, &["add", "user.txt"]);
        run_git(&main, &["commit", "-m", "add user.txt"]);
        let before_refs = git_stdout(&main, &["show-ref"]);

        let (store, _rx, run) = open_real_store_and_preview(
            &main,
            &src,
            FinishMode::MergeInto,
            true,
            false,
            Arc::new(FakeAgentDispatcher::unavailable()),
        );
        let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
            panic!("expected AwaitingConfirm");
        };
        store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
            run_id: run.id,
            fingerprint,
            running_terminal_paths: Vec::new(),
        }));
        wait_real_run(&store, |r| {
            matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
        });

        std::fs::write(main.join("user.txt"), "NEW USER STAGED DATA\n").expect("edit user.txt");
        run_git(&main, &["add", "user.txt"]);

        store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem {
            run_id: run.id,
        }));
        let done = wait_real_run(&store, |r| !r.is_active());
        assert!(
            matches!(
                done.sources[0].phase,
                ItemPhase::Skipped(SkipReason::UserSkipped)
            ),
            "F4 ({magic}): scoped squash abort must complete as Skipped(UserSkipped), got {:?}",
            done.sources[0].phase
        );
        assert_eq!(
            std::fs::read_to_string(main.join("user.txt")).expect("read user.txt"),
            "NEW USER STAGED DATA\n",
            "F4 ({magic}): unrelated staged user data must NOT be discarded via pathspec expansion"
        );
        assert_eq!(
            git_stdout(&main, &["show-ref"]),
            before_refs,
            "F4 ({magic}): refs must remain unchanged"
        );
        let status = git_stdout(&main, &["status", "--porcelain"]);
        assert_eq!(
            status.trim(),
            "M  user.txt",
            "F4 ({magic}): unrelated tracked change must remain staged in index; status={status:?}"
        );
    }

    // Uncertain ownership: when baseline_entries is unavailable (None), AbortItem MUST stop and
    // preserve both index and working file instead of attempting a broad restore.
    let (_dir, main, src) = setup_real_finish_fixture(true);
    std::fs::write(main.join("user.txt"), "original user content\n").expect("write user.txt");
    run_git(&main, &["add", "user.txt"]);
    run_git(&main, &["commit", "-m", "add user.txt"]);
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        true,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    // Replace conflicted file with an edited version and stage unrelated user.txt -> ownership is
    // no longer cleanly separable -> abort must refuse and preserve both files & index.
    std::fs::write(main.join("base.txt"), "partial manual edit\n").expect("edit base");
    std::fs::write(main.join("user.txt"), "USER STAGED UNTOUCHED\n").expect("edit user");
    run_git(&main, &["add", "user.txt"]);
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem {
        run_id: run.id,
    }));
    let refused = wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::Failed { .. })
    });
    assert_eq!(refused.phase, RunPhase::Paused { current: 0 });
    assert_eq!(
        std::fs::read_to_string(main.join("base.txt")).expect("read base"),
        "partial manual edit\n"
    );
    assert_eq!(
        std::fs::read_to_string(main.join("user.txt")).expect("read user"),
        "USER STAGED UNTOUCHED\n"
    );
    assert!(
        git_stdout(&main, &["status", "--porcelain"]).contains("M  user.txt"),
        "F4: index must remain intact when abort stops on uncertain ownership"
    );

    // Uncertain ownership (conflict_baseline_entries = None): D/F conflict leaves a directory at a
    // tracked conflict path so `dirty_entries` is unverified (`None`). `SkipItem` must stop in
    // `Failed`/`Paused` and preserve both staged user data and working files.
    let (_dir_df, main_df, src_df) = setup_real_finish_fixture(false);
    std::fs::write(main_df.join("df_entry"), "main df file\n").expect("write main df_entry");
    std::fs::write(main_df.join("user.txt"), "original user content\n").expect("write user.txt");
    run_git(&main_df, &["add", "df_entry", "user.txt"]);
    run_git(&main_df, &["commit", "-m", "main df file"]);

    std::fs::create_dir_all(src_df.join("df_entry")).expect("mkdir src df_entry");
    std::fs::write(src_df.join("df_entry/child.txt"), "source df child\n")
        .expect("write src df_entry/child.txt");
    run_git(&src_df, &["add", "df_entry/child.txt"]);
    run_git(&src_df, &["commit", "-m", "feat df dir"]);

    let (store_df, _rx_df, run_df) = open_real_store_and_preview(
        &main_df,
        &src_df,
        FinishMode::MergeInto,
        true,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint: fp_df } = run_df.phase else {
        panic!("expected AwaitingConfirm");
    };
    store_df.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run_df.id,
        fingerprint: fp_df,
        running_terminal_paths: Vec::new(),
    }));
    let paused_df = wait_real_run(&store_df, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    assert!(
        paused_df.sources[0].conflict_baseline_entries.is_none(),
        "F4: directory-vs-file conflict must capture conflict_baseline_entries as None"
    );
    std::fs::write(main_df.join("user.txt"), "USER STAGED DURING DF CONFLICT\n")
        .expect("edit user.txt");
    run_git(&main_df, &["add", "user.txt"]);
    store_df.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem {
        run_id: run_df.id,
    }));
    let refused_df = wait_real_run(&store_df, |r| {
        matches!(r.sources[0].phase, ItemPhase::Failed { .. })
    });
    assert_eq!(refused_df.phase, RunPhase::Paused { current: 0 });
    assert_eq!(
        std::fs::read_to_string(main_df.join("user.txt")).expect("read user.txt"),
        "USER STAGED DURING DF CONFLICT\n",
        "F4: uncertain ownership SkipItem must preserve working file"
    );
    assert!(
        git_stdout(&main_df, &["status", "--porcelain"]).contains("M  user.txt"),
        "F4: uncertain ownership SkipItem must preserve staged user index entry"
    );
    assert!(
        main_df.join(".git/SQUASH_MSG").is_file(),
        "F4: refused abort with None baseline must preserve SQUASH_MSG"
    );
}

struct SlowRecordingAgent {
    delay: Duration,
    calls: Mutex<Vec<PathBuf>>,
    launch_result: Mutex<Box<dyn Fn() -> gitcomet_core::services::Result<AgentLaunch> + Send>>,
}

impl SlowRecordingAgent {
    fn ok_after(delay: Duration, session_id: Option<&str>) -> Self {
        let sid = session_id.map(ToOwned::to_owned);
        Self {
            delay,
            calls: Mutex::new(Vec::new()),
            launch_result: Mutex::new(Box::new(move || {
                Ok(AgentLaunch {
                    session_id: sid.clone(),
                    operation_id: Some("op_recorded_1".to_string()),
                    outcome_kind: Some("launched".to_string()),
                    detail: None,
                })
            })),
        }
    }

    fn err_after(delay: Duration, msg: &str) -> Self {
        let msg = msg.to_string();
        Self {
            delay,
            calls: Mutex::new(Vec::new()),
            launch_result: Mutex::new(Box::new(move || {
                Err(Error::new(ErrorKind::Backend(msg.clone())))
            })),
        }
    }
}

impl AgentDispatcher for SlowRecordingAgent {
    fn availability(&self) -> AgentDispatchAvailability {
        AgentDispatchAvailability::Available {
            executable: PathBuf::from("fixture-only"),
        }
    }

    fn installed(
        &self,
        _workspace: &Path,
        _cancellation: &CancellationToken,
    ) -> gitcomet_core::services::Result<Vec<InstalledAgent>> {
        Ok(vec![InstalledAgent {
            id: "fake".to_string(),
            name: "Fake".to_string(),
            is_default: true,
        }])
    }

    fn launch(
        &self,
        workspace: &Path,
        _agent_id: &str,
        _prompt: &str,
        _retry_operation_id: Option<&str>,
    ) -> gitcomet_core::services::Result<AgentLaunch> {
        self.calls.lock().unwrap().push(workspace.to_path_buf());
        std::thread::sleep(self.delay);
        (self.launch_result.lock().unwrap())()
    }

    fn sessions(
        &self,
        _workspace: &Path,
        _cancellation: &CancellationToken,
    ) -> gitcomet_core::services::Result<Vec<AgentSession>> {
        Ok(Vec::new())
    }
}

#[test]
fn f5_agent_dispatch_blocks_reentrant_launch_and_unknown_receipt_redispatch() {
    // 1. Back-to-back DispatchAgent during 200ms launch must only invoke launch ONCE for both modes.
    for mode in [FinishMode::MergeInto, FinishMode::RebaseOnto] {
        let (_dir, main, src) = setup_real_finish_fixture(true);
        let agent = Arc::new(SlowRecordingAgent::ok_after(
            Duration::from_millis(200),
            Some("sess_unique"),
        ));
        let (store, _rx, run) =
            open_real_store_and_preview(&main, &src, mode, false, false, agent.clone());
        let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
            panic!("expected AwaitingConfirm");
        };
        store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
            run_id: run.id,
            fingerprint,
            running_terminal_paths: Vec::new(),
        }));
        wait_real_run(&store, |r| {
            matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
        });
        store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::DispatchAgent {
            run_id: run.id,
            agent_id: "fake".to_string(),
        }));
        store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::DispatchAgent {
            run_id: run.id,
            agent_id: "fake".to_string(),
        }));
        wait_real_run(&store, |r| {
            matches!(r.sources[0].phase, ItemPhase::AgentDispatched { .. })
        });
        let calls = agent.calls.lock().unwrap().clone();
        assert_eq!(
            calls.len(),
            1,
            "F5 ({mode:?}): duplicate launch must be blocked while in flight"
        );
        let expected_site = if mode == FinishMode::MergeInto {
            &main
        } else {
            &src
        };
        assert_eq!(
            gitcomet_core::path_utils::canonicalize_or_original(calls[0].clone()),
            gitcomet_core::path_utils::canonicalize_or_original(expected_site.clone())
        );
    }

    // 2. Uncertain/lost launch receipt locks out fresh re-dispatch and records `--retry-launch`.
    let (_dir, main, src) = setup_real_finish_fixture(true);
    let lost_agent = Arc::new(SlowRecordingAgent::err_after(
        Duration::from_millis(50),
        "launch reply lost; re-issue with --retry-launch op_lost_42",
    ));
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        false,
        false,
        lost_agent.clone(),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::DispatchAgent {
        run_id: run.id,
        agent_id: "fake".to_string(),
    }));
    let after_err = wait_real_run(&store, |r| {
        r.sources[0].agent_receipt_unknown
            && matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    assert_eq!(
        after_err.sources[0].pending_retry_operation_id.as_deref(),
        Some("op_lost_42")
    );
    // Attempting a second fresh DispatchAgent MUST be refused!
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::DispatchAgent {
        run_id: run.id,
        agent_id: "fake".to_string(),
    }));
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(
        lost_agent.calls.lock().unwrap().len(),
        1,
        "F5: unknown launch receipt must prevent a second fresh launch"
    );

    // 3. Launch returning Ok without exact session_id marks receipt unknown and blocks fresh re-dispatch
    // even after manual Recheck or SwitchToManual; Recheck during AgentLaunching cannot clobber it.
    let (_dir3, main3, src3) = setup_real_finish_fixture(true);
    let no_session_agent = Arc::new(SlowRecordingAgent::ok_after(
        Duration::from_millis(120),
        None,
    ));
    let (store3, _rx3, run3) = open_real_store_and_preview(
        &main3,
        &src3,
        FinishMode::MergeInto,
        false,
        false,
        no_session_agent.clone(),
    );
    let RunPhase::AwaitingConfirm { fingerprint: fp3 } = run3.phase else {
        panic!("expected AwaitingConfirm");
    };
    store3.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run3.id,
        fingerprint: fp3,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store3, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    store3.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::DispatchAgent {
        run_id: run3.id,
        agent_id: "fake".to_string(),
    }));
    // Sending Recheck + second DispatchAgent while AgentLaunching is in flight must not clobber launch!
    store3.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Recheck {
        run_id: run3.id,
    }));
    store3.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::DispatchAgent {
        run_id: run3.id,
        agent_id: "fake".to_string(),
    }));
    let dispatched3 = wait_real_run(&store3, |r| {
        matches!(r.sources[0].phase, ItemPhase::AgentDispatched { .. })
    });
    assert!(dispatched3.sources[0].agent_receipt_unknown);
    store3.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SwitchToManual {
        run_id: run3.id,
    }));
    wait_real_run(&store3, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    store3.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::DispatchAgent {
        run_id: run3.id,
        agent_id: "fake".to_string(),
    }));
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(
        no_session_agent.calls.lock().unwrap().len(),
        1,
        "F5: missing session receipt must block fresh duplicate launch after SwitchToManual"
    );
}

#[test]
fn f8_refresh_preview_after_target_and_source_moved_requires_second_confirmation() {
    // TargetMoved -> RefreshPreview -> AwaitingConfirm with new fingerprint -> no write until Confirm!
    let (_dir, main, src) = setup_real_finish_fixture(false);
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint: fp1 } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    std::fs::write(main.join("late_target.txt"), "late target\n").expect("write late target");
    run_git(&main, &["add", "late_target.txt"]);
    run_git(&main, &["commit", "-m", "late target commit"]);

    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint: fp1,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(
            r.sources[0].phase,
            ItemPhase::NeedsDecision {
                drift: DriftKind::TargetMoved { .. },
                ..
            }
        )
    });

    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::ResolveDrift {
        run_id: run.id,
        choice: DriftResolution::RefreshPreview,
    }));
    let refreshed = wait_real_run(&store, |r| {
        matches!(r.phase, RunPhase::AwaitingConfirm { .. })
    });
    let RunPhase::AwaitingConfirm { fingerprint: fp2 } = refreshed.phase else {
        panic!("expected AwaitingConfirm after RefreshPreview");
    };
    assert_ne!(
        fp1, fp2,
        "F8: refreshed preview must produce a new fingerprint"
    );
    assert!(
        !main.join("feature.txt").exists(),
        "F8: RefreshPreview must NOT merge before second Confirm"
    );

    // Confirm with the new fingerprint completes the merge cleanly.
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint: fp2,
        running_terminal_paths: Vec::new(),
    }));
    let done = wait_real_run(&store, |r| !r.is_active());
    assert!(matches!(
        done.sources[0].phase,
        ItemPhase::Done(ItemOutcome::Merged { .. })
    ));
    assert!(main.join("late_target.txt").exists());
    assert!(main.join("feature.txt").exists());

    // SourceMoved -> RefreshPreview -> AwaitingConfirm with new fingerprint -> no write until second Confirm!
    let (_dir2, main2, src2) = setup_real_finish_fixture(false);
    let main2_before = git_stdout(&main2, &["rev-parse", "refs/heads/main"]);
    let (store2, _rx2, run2) = open_real_store_and_preview(
        &main2,
        &src2,
        FinishMode::MergeInto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint: sfp1 } = run2.phase else {
        panic!("expected AwaitingConfirm");
    };
    std::fs::write(src2.join("late_source.txt"), "late source\n").expect("write late source");
    run_git(&src2, &["add", "late_source.txt"]);
    run_git(&src2, &["commit", "-m", "late source commit"]);

    store2.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run2.id,
        fingerprint: sfp1,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store2, |r| {
        matches!(
            r.sources[0].phase,
            ItemPhase::NeedsDecision {
                drift: DriftKind::SourceMoved { .. },
                ..
            }
        )
    });
    store2.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::ResolveDrift {
        run_id: run2.id,
        choice: DriftResolution::RefreshPreview,
    }));
    let refreshed2 = wait_real_run(&store2, |r| {
        matches!(r.phase, RunPhase::AwaitingConfirm { .. })
    });
    let RunPhase::AwaitingConfirm { fingerprint: sfp2 } = refreshed2.phase else {
        panic!("expected AwaitingConfirm after SourceMoved RefreshPreview");
    };
    assert_ne!(
        sfp1, sfp2,
        "F8: SourceMoved RefreshPreview must produce new fingerprint"
    );
    assert_eq!(
        git_stdout(&main2, &["rev-parse", "refs/heads/main"]),
        main2_before,
        "F8: SourceMoved RefreshPreview must NOT merge before second Confirm"
    );
    store2.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run2.id,
        fingerprint: sfp2,
        running_terminal_paths: Vec::new(),
    }));
    let done2 = wait_real_run(&store2, |r| !r.is_active());
    assert!(matches!(
        done2.sources[0].phase,
        ItemPhase::Done(ItemOutcome::Merged { .. })
    ));
    assert!(main2.join("late_source.txt").exists());
}

#[test]
fn f9_live_terminal_opened_after_confirm_blocks_cleanup_on_real_repo() {
    let (_dir, main, src) = setup_real_finish_fixture(true);
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        false,
        true,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    // Confirm with NO running terminals initially.
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });

    // AFTER Confirm, user opens an embedded terminal in `src`!
    store.set_running_terminal_paths(vec![src.clone()]);

    // Resolve conflict and continue -> cleanup must observe the live terminal and KEEP `src`.
    std::fs::write(main.join("base.txt"), "resolved\n").expect("resolve");
    run_git(&main, &["add", "base.txt"]);
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Recheck {
        run_id: run.id,
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::AwaitingReview { .. })
    });
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Continue {
        run_id: run.id,
    }));
    let done = wait_real_run(&store, |r| !r.is_active());
    assert!(matches!(
        done.sources[0].phase,
        ItemPhase::Done(ItemOutcome::Merged { .. })
    ));
    assert_eq!(
        done.sources[0].cleanup,
        CleanupOutcome::Kept(CleanupBlocker::OpenGitCometTerminal),
        "F9: terminal opened after Confirm must block cleanup"
    );
    assert!(
        src.exists(),
        "F9: source worktree with open terminal must remain on disk"
    );
}

#[test]
fn f10_active_finish_run_blocks_conflicting_mutations_and_allows_conflict_tools() {
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::MergeInto,
        false,
        true,
    );
    assert!(state.worktree_finish_owns_workdir(Path::new("/tmp/repo-main")));
    assert!(state.worktree_finish_owns_workdir(Path::new("/tmp/repo-wt-a")));
    assert!(state.repo_history_rewrite_busy(repo_id));

    // All conflicting mutating messages on protected worktrees must be blocked (zero effects).
    let blocked_msgs = vec![
        Msg::CheckoutBranch {
            repo_id,
            name: "other".to_string(),
        },
        Msg::MergeRef {
            repo_id,
            reference: "feat/a".to_string(),
        },
        Msg::Rebase {
            repo_id,
            onto: "main".to_string(),
        },
        Msg::Reset {
            repo_id,
            target: "HEAD~1".to_string(),
            mode: gitcomet_core::services::ResetMode::Hard,
        },
        Msg::Pull {
            repo_id,
            mode: gitcomet_core::services::PullMode::Default,
        },
        Msg::RemoveWorktree {
            repo_id,
            path: PathBuf::from("/tmp/repo-wt-a"),
            delete_branch: None,
        },
        Msg::RenameBranch {
            repo_id,
            old_name: "feat/a".to_string(),
            new_name: "feat/renamed".to_string(),
            force: false,
        },
        Msg::DeleteBranch {
            repo_id,
            name: "feat/a".to_string(),
        },
        Msg::PruneMergedBranches { repo_id },
    ];
    for msg in blocked_msgs {
        let effects = step_reduce(&mut state, &mut repos, &id_alloc, msg);
        assert!(
            effects.is_empty(),
            "F10: conflicting mutation must be blocked while finish run {run_id:?} owns worktree"
        );
    }

    // Conflict resolution tools (MergeAbort, StagePath) remain allowed per §7.4.
    let abort_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::MergeAbort { repo_id },
    );
    assert!(
        !abort_effects.is_empty(),
        "F10: MergeAbort must remain allowed for manual recovery"
    );
}

#[test]
fn f10_third_linked_worktree_blocks_protected_ref_mutations_allows_unrelated_and_independent_repo()
{
    use gitcomet_core::services::{CheckoutRemoteBranchMode, WorktreeCheckout};

    let (dir, main, src) = setup_real_finish_fixture(true);
    let view = dir.path().join("view");
    run_git(
        &main,
        &["worktree", "add", "-b", "view", view.to_str().unwrap()],
    );
    run_git(&main, &["branch", "unrelated", "main"]);

    // Independent repository with its own `main` branch (different common_dir).
    let indep_dir = tempfile::tempdir().expect("independent repo dir");
    let indep = indep_dir.path().join("indep");
    std::fs::create_dir_all(&indep).expect("mkdir indep");
    run_git(&indep, &["init", "-b", "main"]);
    run_git(&indep, &["config", "user.name", "Tester"]);
    run_git(&indep, &["config", "user.email", "test@example.com"]);
    run_git(&indep, &["config", "commit.gpgsign", "false"]);
    std::fs::write(indep.join("readme.txt"), "indep\n").expect("write indep");
    run_git(&indep, &["add", "readme.txt"]);
    run_git(&indep, &["commit", "-m", "init indep"]);

    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let wait_repo_id = |target_path: &Path| -> RepoId {
        let canonical =
            gitcomet_core::path_utils::canonicalize_or_original(target_path.to_path_buf());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(repo) = store.snapshot().repos.iter().find(|r| {
                gitcomet_core::path_utils::canonicalize_or_original(r.spec.workdir.clone())
                    == canonical
                    && matches!(r.open, Loadable::Ready(()))
            }) {
                return repo.id;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for repo {} to open",
                target_path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    store.dispatch(Msg::OpenRepo(view.clone()));
    let view_id = wait_repo_id(&view);
    store.dispatch(Msg::OpenRepo(indep.clone()));
    let indep_id = wait_repo_id(&indep);

    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });

    let snap = store.snapshot();
    assert!(
        snap.worktree_finish_protects_branch(view_id, "main"),
        "F10: third worktree in same common_dir must protect target branch 'main'"
    );
    assert!(
        snap.worktree_finish_protects_branch(view_id, "refs/heads/main"),
        "F10: full ref name 'refs/heads/main' must also be protected"
    );
    assert!(
        snap.worktree_finish_protects_branch(view_id, "feat"),
        "F10: third worktree in same common_dir must protect source branch 'feat'"
    );
    assert!(
        !snap.worktree_finish_protects_branch(view_id, "view"),
        "F10: unrelated branch 'view' must NOT be marked protected"
    );
    assert!(
        snap.has_active_worktree_finish_run(view_id),
        "F10: third worktree in same common_dir must see active finish run"
    );
    assert!(
        !snap.worktree_finish_protects_branch(indep_id, "main"),
        "F10: independent repo with same branch name 'main' must NOT be blocked"
    );
    assert!(
        !snap.has_active_worktree_finish_run(indep_id),
        "F10: independent repo must NOT see active finish run from another common_dir"
    );

    let before_refs = git_stdout(&main, &["show-ref"]);
    let extra_wt = dir.path().join("extra-wt");
    let blocked_third_tab_msgs = vec![
        Msg::RenameBranch {
            repo_id: view_id,
            old_name: "main".to_string(),
            new_name: "unexpected-renamed".to_string(),
            force: false,
        },
        Msg::RenameBranch {
            repo_id: view_id,
            old_name: "unrelated".to_string(),
            new_name: "main".to_string(),
            force: true,
        },
        Msg::DeleteBranch {
            repo_id: view_id,
            name: "feat".to_string(),
        },
        Msg::ForceDeleteBranch {
            repo_id: view_id,
            name: "feat".to_string(),
        },
        Msg::DeleteBranches {
            repo_id: view_id,
            names: vec!["unrelated".to_string(), "main".to_string()],
            force: true,
        },
        Msg::PruneMergedBranches { repo_id: view_id },
        Msg::CheckoutBranch {
            repo_id: view_id,
            name: "main".to_string(),
        },
        Msg::CreateBranch {
            repo_id: view_id,
            name: "main".to_string(),
            target: "view".to_string(),
        },
        Msg::CreateBranchAndCheckout {
            repo_id: view_id,
            name: "main".to_string(),
            target: "view".to_string(),
            force: true,
        },
        Msg::CheckoutRemoteBranch {
            repo_id: view_id,
            remote: "origin".to_string(),
            branch: "main".to_string(),
            local_branch: "main".to_string(),
            mode: CheckoutRemoteBranchMode::Overwrite,
        },
        Msg::AddWorktree {
            repo_id: view_id,
            path: extra_wt.clone(),
            checkout: WorktreeCheckout::Existing {
                reference: "main".to_string(),
            },
            open_after: false,
        },
        Msg::RemoveWorktree {
            repo_id: view_id,
            path: extra_wt,
            delete_branch: Some("feat".to_string()),
        },
        Msg::Fetch(crate::msg::FetchMsg::Refspecs {
            repo_id: view_id,
            remote: ".".to_string(),
            refspecs: vec!["+feat:refs/heads/main".to_string()],
        }),
        Msg::Fetch(crate::msg::FetchMsg::Refspecs {
            repo_id: view_id,
            remote: ".".to_string(),
            refspecs: vec!["+feat:heads/main".to_string()],
        }),
        Msg::Fetch(crate::msg::FetchMsg::Refspecs {
            repo_id: view_id,
            remote: ".".to_string(),
            refspecs: vec!["+refs/heads/*:refs/*".to_string()],
        }),
    ];
    for msg in blocked_third_tab_msgs {
        store.dispatch(msg);
    }
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(
        git_stdout(&main, &["show-ref"]),
        before_refs,
        "F10: all protected ref mutations via third linked worktree must be blocked"
    );
    assert_eq!(
        git_stdout(&main, &["branch", "--show-current"]),
        "main",
        "F10: target worktree must remain on branch 'main'"
    );

    // Unrelated branch rename via third worktree MUST still work!
    store.dispatch(Msg::RenameBranch {
        repo_id: view_id,
        old_name: "unrelated".to_string(),
        new_name: "unrelated-renamed".to_string(),
        force: false,
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let refs = git_stdout(&main, &["show-ref"]);
        if refs.contains("refs/heads/unrelated-renamed") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "F10: unrelated branch rename via third worktree should remain allowed; refs:\n{refs}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // Independent repository's own `main` branch MUST NOT be falsely blocked!
    store.dispatch(Msg::RenameBranch {
        repo_id: indep_id,
        old_name: "main".to_string(),
        new_name: "indep-renamed".to_string(),
        force: false,
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if git_stdout(&indep, &["branch", "--show-current"]) == "indep-renamed" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "F10: independent repo's branch named 'main' must NOT be blocked"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    // Also verify RebaseOnto protects the target branch's open worktree tab via head_branch
    // even when TargetHost is NotNeeded, and ResolveBranchExistsPrompt / Undo touching a
    // protected branch from a third worktree are blocked in reducer state.
    let (mut state, mut repos, id_alloc, main_repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let shared_common: Arc<Path> = Arc::from(Path::new("/tmp/repo-main/.git"));
    state.repos[0].common_dir = Some(Arc::clone(&shared_common));
    state.repos[0].head_branch = Loadable::Ready("main".to_string());
    let third_id = RepoId(2);
    let mut third_repo = RepoState::new_opening(
        third_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo-wt-view"),
        },
    );
    third_repo.common_dir = Some(Arc::clone(&shared_common));
    third_repo.head_branch = Loadable::Ready("view".to_string());
    state.repos.push(third_repo);

    let (_rebase_run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        main_repo_id,
        FinishMode::RebaseOnto,
        false,
        false,
    );
    assert!(
        state.worktree_finish_protects_repo_head(main_repo_id),
        "F10: target branch tab must be protected during RebaseOnto"
    );
    assert!(
        !state.worktree_finish_protects_repo_head(third_id),
        "F10: third worktree on unrelated branch 'view' must not have workdir blocked"
    );
    assert!(
        step_reduce(
            &mut state,
            &mut repos,
            &id_alloc,
            Msg::Reset {
                repo_id: main_repo_id,
                target: "HEAD~1".to_string(),
                mode: gitcomet_core::services::ResetMode::Hard,
            },
        )
        .is_empty(),
        "F10: Reset on target branch tab during RebaseOnto must be blocked"
    );
    assert!(
        step_reduce(
            &mut state,
            &mut repos,
            &id_alloc,
            Msg::ResolveBranchExistsPrompt {
                prompt: crate::model::BranchExistsPromptState {
                    repo_id: third_id,
                    name: "other".to_string(),
                    target: "main".to_string(),
                    operation: crate::model::BranchExistsPromptOperation::RenameBranch {
                        old_name: "main".to_string(),
                    },
                },
                choice: crate::msg::BranchExistsChoice::OverwriteAndCheckout,
            },
        )
        .is_empty(),
        "F10: ResolveBranchExistsPrompt renaming protected branch from third tab must be blocked"
    );
    assert!(
        step_reduce(
            &mut state,
            &mut repos,
            &id_alloc,
            Msg::RemoveWorktree {
                repo_id: third_id,
                path: PathBuf::from("/tmp/repo-main"),
                delete_branch: None,
            },
        )
        .is_empty(),
        "F10: RemoveWorktree targeting protected target branch worktree from third tab during RebaseOnto must be blocked"
    );
    assert!(
        !step_reduce(
            &mut state,
            &mut repos,
            &id_alloc,
            Msg::Fetch(crate::msg::FetchMsg::Refspecs {
                repo_id: third_id,
                remote: "origin".to_string(),
                refspecs: vec!["+refs/heads/main:refs/remotes/origin/main".to_string()],
            }),
        )
        .is_empty(),
        "F10: fetching into remote-tracking ref from third tab must remain allowed"
    );
    assert!(
        !step_reduce(
            &mut state,
            &mut repos,
            &id_alloc,
            Msg::Fetch(crate::msg::FetchMsg::Refspecs {
                repo_id: third_id,
                remote: "origin".to_string(),
                refspecs: vec!["+refs/heads/unrelated:refs/heads/unrelated".to_string()],
            }),
        )
        .is_empty(),
        "F10: fetching into unrelated local branch from third tab must remain allowed"
    );
}

#[test]
fn external_rebase_adopt_and_squash_range_credential_retry() {
    // 1. External rebase completion adoption on real temp repo (target stays unchanged).
    let (_dir, main, src) = setup_real_finish_fixture(true);
    let main_before = git_stdout(&main, &["rev-parse", "refs/heads/main"]);
    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::RebaseOnto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    // Resolve and finish rebase EXTERNALLY.
    std::fs::write(src.join("base.txt"), "external rebase finish\n").expect("resolve");
    run_git(&src, &["add", "base.txt"]);
    let status = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(&src)
        .env("GIT_EDITOR", "true")
        .args(["rebase", "--continue"])
        .status()
        .expect("external rebase continue");
    assert!(status.success());
    let ext_head = git_stdout(&src, &["rev-parse", "HEAD"]);

    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Recheck {
        run_id: run.id,
    }));
    wait_real_run(&store, |r| {
        matches!(
            r.sources[0].phase,
            ItemPhase::NeedsDecision {
                drift: DriftKind::AdoptCommit { .. },
                ..
            }
        )
    });
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::ResolveDrift {
        run_id: run.id,
        choice: DriftResolution::AdoptCommit,
    }));
    let done = wait_real_run(&store, |r| !r.is_active());
    assert!(matches!(
        done.sources[0].phase,
        ItemPhase::Done(ItemOutcome::Rebased { .. })
    ));
    assert_eq!(git_stdout(&src, &["rev-parse", "HEAD"]), ext_head);
    assert_eq!(
        git_stdout(&main, &["rev-parse", "refs/heads/main"]),
        main_before
    );

    // 2. SquashRange credential retry re-emits FinishStep::SquashRange with preserved range.
    let (mut state, mut repos, id_alloc, repo_id) =
        setup_state_with_repo(Path::new("/tmp/repo-main"));
    let (run_id, _) = preview_and_confirm_single(
        &mut state,
        &mut repos,
        &id_alloc,
        repo_id,
        FinishMode::RebaseOnto,
        true,
        false,
    );
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::ItemPreflightLoaded {
                run_id,
                item: 0,
                target_head: Some(cid("target_0")),
                target_snapshot: None,
                source_head: Some(cid("src_a_1")),
                source_snapshot: Some(clean_snap("feat/a", "src_a_1")),
                expected_merged_tree: None,
                error: None,
            },
        ))),
    );
    let rebased_snap = clean_snap("feat/a", "rebased_head_2");
    let squash_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::Rebase,
                before: clean_snap("feat/a", "src_a_1"),
                after: Some(rebased_snap.clone()),
                command: Ok(FinishStepCommandOutput::from_output(
                    CommandOutput::empty_success("git rebase"),
                )),
                precheck_violation: None,
                rebased_range: Some((cid("rebased_oldest_1"), 2)),
                target_is_ancestor: Some(true),
            },
        ))),
    );
    assert!(matches!(
        squash_effects.first(),
        Some(Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
            step: FinishStep::SquashRange,
            ..
        }))
    ));
    let signing_err = Error::new(ErrorKind::Git(
        GitFailure::new(
            "git commit",
            GitFailureId::CommandFailed,
            Some(128),
            Vec::new(),
            b"Enter passphrase for key '/tmp/id_ed25519':".to_vec(),
            Some("Enter passphrase for key '/tmp/id_ed25519':".to_string()),
        )
        .with_auth_outcome(AuthOutcome::NeedsInput(AuthChallenge::from_prompt(
            GitAuthKind::Passphrase,
            "Enter passphrase for key '/tmp/id_ed25519':",
        ))),
    ));
    step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(
            WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item: 0,
                step: FinishStep::SquashRange,
                before: rebased_snap,
                after: None,
                command: Err(signing_err),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            },
        ))),
    );
    assert!(matches!(
        state.worktree_finish_runs[0].sources[0].phase,
        ItemPhase::AwaitingCredential {
            step: FinishStep::SquashRange,
            ..
        }
    ));
    let retry_effects = step_reduce(
        &mut state,
        &mut repos,
        &id_alloc,
        Msg::WorktreeFinish(WorktreeFinishMsg::RetryCredential { run_id }),
    );
    assert!(matches!(
        retry_effects.first(),
        Some(Effect::WorktreeFinish(WorktreeFinishEffect::RunStep {
            step: FinishStep::SquashRange,
            action: FinishStepAction::SquashRange { oldest, expected_head, .. },
            ..
        })) if oldest == &cid("rebased_oldest_1") && expected_head == &cid("rebased_head_2")
    ));
}

/// F4': skipping a squash conflict whose owned parent path must turn from a directory back
/// into a file refuses when the directory holds data the squash does not own (here an
/// ignored file written after the pause). The item fails for manual recovery instead of
/// reporting `Skipped`, and the index, conflict state, SQUASH_MSG and user data stay put.
#[test]
fn f4_skip_squash_conflict_refuses_directory_to_file_restore_over_ignored_descendant() {
    let (_dir, main, src) = setup_real_finish_fixture(true);
    for path in [&main, &src] {
        std::fs::write(path.join("slot"), "same contents for rename detection\n")
            .expect("write slot");
        run_git(path, &["add", "slot"]);
        run_git(path, &["commit", "-m", "add slot"]);
    }
    std::fs::remove_file(src.join("slot")).expect("remove slot");
    std::fs::create_dir(src.join("slot")).expect("mkdir slot");
    std::fs::write(
        src.join("slot/owned.txt"),
        "same contents for rename detection\n",
    )
    .expect("write slot/owned.txt");
    run_git(&src, &["add", "-A"]);
    run_git(&src, &["commit", "-m", "move slot under its old path"]);
    let before_refs = git_stdout(&main, &["show-ref"]);

    let (store, _rx, run) = open_real_store_and_preview(
        &main,
        &src,
        FinishMode::MergeInto,
        true,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });

    let ignored = main.join("slot/local.ignored");
    std::fs::write(&ignored, "ignored data written after the pause\n").expect("write ignored");
    let status_args = [
        "status",
        "--porcelain=v2",
        "--ignored",
        "--untracked-files=all",
    ];
    let before_status = git_stdout(&main, &status_args);
    assert!(
        before_status.contains("R100 slot/owned.txt\tslot")
            && before_status.contains("! slot/local.ignored"),
        "expected the directory/file rename conflict: {before_status}"
    );
    let before_index = git_stdout(&main, &["ls-files", "--stage"]);

    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::SkipItem {
        run_id: run.id,
    }));
    let done = wait_real_run(&store, |r| {
        !r.is_active() || matches!(r.sources[0].phase, ItemPhase::Failed { .. })
    });
    assert!(
        matches!(done.sources[0].phase, ItemPhase::Failed { .. }),
        "F4': an unprovable restore must ask for manual recovery, not report Skipped: {:?}",
        done.sources[0].phase
    );
    assert_eq!(
        std::fs::read_to_string(&ignored).expect("ignored data preserved"),
        "ignored data written after the pause\n"
    );
    assert!(main.join("slot").is_dir());
    assert_eq!(git_stdout(&main, &status_args), before_status);
    assert_eq!(git_stdout(&main, &["ls-files", "--stage"]), before_index);
    assert!(main.join(".git/SQUASH_MSG").is_file());
    assert_eq!(git_stdout(&main, &["show-ref"]), before_refs);
}

/// F10': while a run protects the unchecked-out target `release`, fetches and pulls from any
/// worktree of the repository refuse a remote whose configured refspec writes
/// `refs/heads/release`, naming the reason, while a remote that only writes
/// `refs/remotes/*` still updates.
#[test]
fn f10_configured_fetch_refspec_cannot_move_protected_branch_from_third_worktree() {
    let (dir, main, src) = setup_real_finish_fixture(true);
    run_git(&main, &["branch", "release"]);
    let view = dir.path().join("view");
    run_git(
        &main,
        &["worktree", "add", "-b", "view", view.to_str().unwrap()],
    );
    let remote = dir.path().join("remote");
    run_git(
        dir.path(),
        &[
            "clone",
            "--no-hardlinks",
            main.to_str().unwrap(),
            remote.to_str().unwrap(),
        ],
    );
    run_git(&remote, &["config", "user.email", "test@example.com"]);
    run_git(&remote, &["config", "user.name", "Tester"]);
    run_git(&remote, &["config", "commit.gpgsign", "false"]);
    std::fs::write(remote.join("advanced.txt"), "remote only\n").expect("write remote");
    run_git(&remote, &["add", "advanced.txt"]);
    run_git(&remote, &["commit", "-m", "remote advances"]);
    let remote_head = git_stdout(&remote, &["rev-parse", "main"]);
    for name in ["danger", "safe"] {
        run_git(&main, &["remote", "add", name, remote.to_str().unwrap()]);
    }
    run_git(
        &main,
        &[
            "config",
            "--replace-all",
            "remote.danger.fetch",
            "+refs/heads/main:refs/heads/release",
        ],
    );

    let (store, _rx, run) = open_real_store_and_preview_onto(
        &main,
        &src,
        "release",
        FinishMode::RebaseOnto,
        false,
        false,
        Arc::new(FakeAgentDispatcher::unavailable()),
    );
    store.dispatch(Msg::OpenRepo(view.clone()));
    let canonical_view = gitcomet_core::path_utils::canonicalize_or_original(view.clone());
    let deadline = Instant::now() + Duration::from_secs(10);
    let view_id = loop {
        if let Some(repo) = store.snapshot().repos.iter().find(|r| {
            gitcomet_core::path_utils::canonicalize_or_original(r.spec.workdir.clone())
                == canonical_view
                && matches!(r.open, Loadable::Ready(()))
        }) {
            break repo.id;
        }
        assert!(Instant::now() < deadline, "timed out opening view");
        std::thread::sleep(Duration::from_millis(10));
    };
    let RunPhase::AwaitingConfirm { fingerprint } = run.phase else {
        panic!("expected AwaitingConfirm");
    };
    store.dispatch(Msg::WorktreeFinish(WorktreeFinishMsg::Confirm {
        run_id: run.id,
        fingerprint,
        running_terminal_paths: Vec::new(),
    }));
    wait_real_run(&store, |r| {
        matches!(r.sources[0].phase, ItemPhase::PausedConflict { .. })
    });
    assert!(
        store
            .snapshot()
            .worktree_finish_protects_branch(view_id, "release")
    );
    let release = git_stdout(&main, &["rev-parse", "release"]);

    // Runs `msg` from the third worktree and returns its command-log entry.
    let run_command = |msg: Msg| {
        let seq = |store: &AppStore| {
            store
                .snapshot()
                .repos
                .iter()
                .find(|r| r.id == view_id)
                .map(|r| r.feedback.command_log_seq)
                .unwrap_or_default()
        };
        let before = seq(&store);
        store.dispatch(msg);
        let deadline = Instant::now() + Duration::from_secs(15);
        while seq(&store) == before {
            assert!(Instant::now() < deadline, "command never finished");
            std::thread::sleep(Duration::from_millis(15));
        }
        let snapshot = store.snapshot();
        let repo = snapshot.repos.iter().find(|r| r.id == view_id).unwrap();
        repo.feedback
            .command_log
            .last()
            .cloned()
            .expect("command log entry")
    };
    let refused = [
        Msg::Fetch(crate::msg::FetchMsg::Remote {
            repo_id: view_id,
            remote: "danger".to_string(),
        }),
        Msg::Fetch(crate::msg::FetchMsg::All { repo_id: view_id }),
        Msg::Fetch(crate::msg::FetchMsg::Refspecs {
            repo_id: view_id,
            remote: "danger".to_string(),
            refspecs: vec!["refs/heads/main".to_string()],
        }),
        Msg::PullBranch {
            repo_id: view_id,
            remote: "danger".to_string(),
            branch: "main".to_string(),
        },
        Msg::Pull {
            repo_id: view_id,
            mode: gitcomet_core::services::PullMode::Default,
        },
    ];
    for msg in refused {
        let label = format!("{msg:?}");
        let entry = run_command(msg);
        assert!(!entry.ok, "{label} must be refused: {entry:?}");
        assert!(
            format!("{entry:?}").contains("configured fetch refspec writes 'refs/heads/release'"),
            "{label} must explain the refusal: {entry:?}"
        );
        assert_eq!(
            git_stdout(&main, &["rev-parse", "release"]),
            release,
            "{label} moved the protected branch"
        );
    }

    let entry = run_command(Msg::Fetch(crate::msg::FetchMsg::Remote {
        repo_id: view_id,
        remote: "safe".to_string(),
    }));
    assert!(
        entry.ok,
        "a remote-tracking fetch must still run: {entry:?}"
    );
    assert_eq!(
        git_stdout(&main, &["rev-parse", "refs/remotes/safe/main"]),
        remote_head
    );
    assert_eq!(git_stdout(&main, &["rev-parse", "release"]), release);
}
