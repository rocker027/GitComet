//! Background effect scheduler for the Worktree Finish workflow.

use super::super::executor::TaskExecutor;
use super::super::repository_runtime::RepositoryRuntimes;
use super::super::worker_channel::StoreWorkerSender;
use super::util::send_or_log;
use crate::model::{
    AppState, FinishMode, FinishSourceRequest, FinishStep, RepoId, TargetHost, WorktreeFinishRunId,
};
use crate::msg::{
    AbortSafetyBaseline, AgentCheckStatus, FinishStepAction, InternalMsg, Msg,
    SourceCleanupCheckResult, SourcePreviewResult, StepSiteGuard, WorktreeFinishEffect,
    WorktreeFinishInternalMsg,
};
use gitcomet_core::auth::{ScopedStagedGitAuth, StagedGitAuth};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{
    CancellationToken, GitBackend, GitRepository, ResetMode, WorktreeCheckout,
};
use gitcomet_core::worktree_finish::{
    AgentDispatchAvailability, AgentDispatcher, FinishOperation, FinishSnapshot,
    FinishStepCommandOutput, IgnoredFilesFingerprint, WorktreeEntryDigest,
    default_squash_merge_message,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule(
    executor: &TaskExecutor,
    backend: &Arc<dyn GitBackend>,
    agent_dispatcher: &Arc<dyn AgentDispatcher>,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    effect: WorktreeFinishEffect,
) {
    match effect {
        WorktreeFinishEffect::LoadPreview {
            run_id,
            origin_repo_id,
            start_index,
            target_branch,
            mode,
            sources,
        } => {
            let Some(origin_repo) = repos.repository(&origin_repo_id).cloned() else {
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::PreviewLoaded {
                        run_id,
                        start_index,
                        common_dir: PathBuf::new(),
                        target_head: None,
                        target_host: TargetHost::NotNeeded,
                        target_snapshot: None,
                        sources: Vec::new(),
                        error: Some("Origin repository is no longer open.".to_string()),
                    },
                );
                return;
            };
            let open_tabs = snapshot_open_tabs(thread_state, repos);
            let backend = Arc::clone(backend);
            let dispatcher = Arc::clone(agent_dispatcher);
            executor.spawn(move || {
                let msg = run_load_preview(
                    run_id,
                    origin_repo_id,
                    start_index,
                    origin_repo,
                    open_tabs,
                    backend,
                    dispatcher,
                    target_branch,
                    mode,
                    sources,
                );
                send_finish(&msg_tx, msg);
            });
        }
        WorktreeFinishEffect::PrepareTempWorktree {
            run_id,
            origin_repo_id,
            target_branch,
            temp_path,
        } => {
            let Some(origin_repo) = repos.repository(&origin_repo_id).cloned() else {
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::TempWorktreePrepared {
                        run_id,
                        path: temp_path,
                        result: Err(Error::new(ErrorKind::Backend(
                            "Origin repository is no longer open.".to_string(),
                        ))),
                    },
                );
                return;
            };
            let backend = Arc::clone(backend);
            executor.spawn(move || {
                let result = (|| {
                    let gate = super::super::mutation_gate::for_repo(origin_repo.as_ref());
                    let _write = gate.write().unwrap_or_else(|e| e.into_inner());
                    let _lease = origin_repo.acquire_operation_lease()?;
                    if let Some(parent) = temp_path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
                    }
                    origin_repo.add_worktree_with_output(
                        &temp_path,
                        &WorktreeCheckout::Existing {
                            reference: target_branch,
                        },
                    )?;
                    let temp_repo = backend.open(&temp_path)?;
                    temp_repo.finish_snapshot(&CancellationToken::new())
                })();
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::TempWorktreePrepared {
                        run_id,
                        path: temp_path,
                        result,
                    },
                );
            });
        }
        WorktreeFinishEffect::PreflightItem {
            run_id,
            origin_repo_id,
            item,
            expected_common_dir,
            mode,
            target_branch,
            target_host,
            source_path,
            source_branch,
            locked_source_head,
            compute_expected_tree,
        } => {
            let Some(origin_repo) = repos.repository(&origin_repo_id).cloned() else {
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::ItemPreflightLoaded {
                        run_id,
                        item,
                        target_head: None,
                        target_snapshot: None,
                        source_head: None,
                        source_snapshot: None,
                        expected_merged_tree: None,
                        error: Some("Origin repository is no longer open.".to_string()),
                    },
                );
                return;
            };
            let open_tabs = snapshot_open_tabs(thread_state, repos);
            let backend = Arc::clone(backend);
            executor.spawn(move || {
                let msg = run_preflight_item(
                    run_id,
                    origin_repo,
                    open_tabs,
                    backend,
                    item,
                    expected_common_dir,
                    mode,
                    target_branch,
                    target_host,
                    source_path,
                    source_branch,
                    locked_source_head,
                    compute_expected_tree,
                );
                send_finish(&msg_tx, msg);
            });
        }
        WorktreeFinishEffect::RunStep {
            run_id,
            origin_repo_id,
            item,
            step,
            site,
            guard,
            action,
            verify_rebase_target,
            auth,
        } => {
            let Some(origin_repo) = repos.repository(&origin_repo_id).cloned() else {
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::StepFinished {
                        run_id,
                        item,
                        step,
                        before: Default::default(),
                        after: None,
                        command: Err(Error::new(ErrorKind::Backend(
                            "Origin repository is no longer open.".to_string(),
                        ))),
                        precheck_violation: None,
                        rebased_range: None,
                        target_is_ancestor: None,
                    },
                );
                return;
            };
            let open_tabs = snapshot_open_tabs(thread_state, repos);
            let backend = Arc::clone(backend);
            executor.spawn(move || {
                let msg = run_step(
                    run_id,
                    origin_repo,
                    open_tabs,
                    backend,
                    item,
                    step,
                    site,
                    guard,
                    action,
                    verify_rebase_target,
                    auth,
                );
                send_finish(&msg_tx, msg);
            });
        }
        WorktreeFinishEffect::RecheckSite {
            run_id,
            origin_repo_id,
            item,
            site,
            verify_rebase_target,
        } => {
            let Some(origin_repo) = repos.repository(&origin_repo_id).cloned() else {
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::RecheckLoaded {
                        run_id,
                        item,
                        snapshot: Err(Error::new(ErrorKind::Backend(
                            "Origin repository is no longer open.".to_string(),
                        ))),
                        target_is_ancestor: None,
                        rebased_range: None,
                    },
                );
                return;
            };
            let open_tabs = snapshot_open_tabs(thread_state, repos);
            let backend = Arc::clone(backend);
            executor.spawn(move || {
                let (snapshot, target_is_ancestor, rebased_range) =
                    match resolve_site_repo(&origin_repo, &open_tabs, &backend, &site) {
                        Ok(repo) => {
                            let snap = repo.finish_snapshot(&CancellationToken::new());
                            let (ancestor, range) = match (&snap, &verify_rebase_target) {
                                (Ok(s), Some(target)) => {
                                    let anc = s
                                        .head
                                        .as_ref()
                                        .and_then(|h| repo.is_commit_ancestor(target, h).ok());
                                    let rng = if anc == Some(true) {
                                        s.head
                                            .as_ref()
                                            .and_then(|h| repo.linear_range_after(target, h).ok())
                                            .flatten()
                                    } else {
                                        None
                                    };
                                    (anc, rng)
                                }
                                _ => (None, None),
                            };
                            (snap, ancestor, range)
                        }
                        Err(err) => (Err(err), None, None),
                    };
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::RecheckLoaded {
                        run_id,
                        item,
                        snapshot,
                        target_is_ancestor,
                        rebased_range,
                    },
                );
            });
        }
        WorktreeFinishEffect::LoadAgents { run_id, site } => {
            let dispatcher = Arc::clone(agent_dispatcher);
            executor.spawn(move || {
                let availability = dispatcher.availability();
                let agents = match &availability {
                    AgentDispatchAvailability::Available { .. } => {
                        dispatcher.installed(&site, &CancellationToken::new())
                    }
                    AgentDispatchAvailability::NotFound => Err(Error::new(ErrorKind::Backend(
                        "Orca CLI was not found.".to_string(),
                    ))),
                    AgentDispatchAvailability::Failed(msg) => {
                        Err(Error::new(ErrorKind::Backend(msg.clone())))
                    }
                };
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::AgentsLoaded {
                        run_id,
                        availability,
                        agents,
                    },
                );
            });
        }
        WorktreeFinishEffect::LaunchAgent {
            run_id,
            item,
            site,
            agent_id,
            request_id,
            prompt,
            retry_operation_id,
        } => {
            let dispatcher = Arc::clone(agent_dispatcher);
            executor.spawn(move || {
                let result =
                    dispatcher.launch(&site, &agent_id, &prompt, retry_operation_id.as_deref());
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::AgentLaunched {
                        run_id,
                        item,
                        agent_id,
                        request_id,
                        result,
                    },
                );
            });
        }
        WorktreeFinishEffect::PollAgent { run_id, item, site } => {
            let dispatcher = Arc::clone(agent_dispatcher);
            executor.spawn(move || {
                let sessions = dispatcher.sessions(&site, &CancellationToken::new());
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::AgentPolled {
                        run_id,
                        item,
                        sessions,
                    },
                );
            });
        }
        WorktreeFinishEffect::CheckCleanup {
            run_id,
            origin_repo_id,
            items,
        } => {
            let Some(origin_repo) = repos.repository(&origin_repo_id).cloned() else {
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::CleanupChecked {
                        run_id,
                        results: Vec::new(),
                        orca_unavailable: true,
                        live_terminal_paths: Vec::new(),
                    },
                );
                return;
            };
            let open_tabs = snapshot_open_tabs(thread_state, repos);
            let backend = Arc::clone(backend);
            let dispatcher = Arc::clone(agent_dispatcher);
            let live_terminals = Arc::clone(&msg_tx.running_terminal_paths);
            executor.spawn(move || {
                let avail = dispatcher.availability();
                let orca_available = matches!(avail, AgentDispatchAvailability::Available { .. });
                let mut results = Vec::with_capacity(items.len());
                for (index, path) in items {
                    let snapshot = resolve_site_repo(&origin_repo, &open_tabs, &backend, &path)
                        .and_then(|repo| repo.finish_snapshot(&CancellationToken::new()));
                    let active_sessions = if orca_available {
                        dispatcher
                            .sessions(&path, &CancellationToken::new())
                            .map_err(|err| AgentCheckStatus::QueryFailed(err.to_string()))
                    } else {
                        Err(AgentCheckStatus::OrcaNotFound)
                    };
                    results.push(SourceCleanupCheckResult {
                        index,
                        snapshot,
                        active_sessions,
                    });
                }
                let live_terminal_paths = live_terminals
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::CleanupChecked {
                        run_id,
                        results,
                        orca_unavailable: !orca_available,
                        live_terminal_paths,
                    },
                );
            });
        }
        WorktreeFinishEffect::RemoveWorktree {
            run_id,
            origin_repo_id,
            item,
            path,
            expected_common_dir,
            expected_branch,
            expected_head,
            expected_ignored_fingerprint,
        } => {
            let Some(origin_repo) = repos.repository(&origin_repo_id).cloned() else {
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::WorktreeRemoved {
                        run_id,
                        item,
                        result: Err(Error::new(ErrorKind::Backend(
                            "Origin repository is no longer open.".to_string(),
                        ))),
                    },
                );
                return;
            };
            let backend = Arc::clone(backend);
            let live_terminals = Arc::clone(&msg_tx.running_terminal_paths);
            executor.spawn(move || {
                let result = (|| {
                    let gate = super::super::mutation_gate::for_repo(origin_repo.as_ref());
                    let _write = gate.write().unwrap_or_else(|e| e.into_inner());
                    let _lease = origin_repo.acquire_operation_lease()?;

                    // F9: re-check live built-in terminal occupancy right inside the mutation gate.
                    let canonical_path = canonicalize_or_original(path.clone());
                    let terminal_in_use = live_terminals
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .iter()
                        .any(|p| canonicalize_or_original(p.clone()) == canonical_path);
                    if terminal_in_use {
                        return Err(Error::new(ErrorKind::Backend(
                            "A built-in terminal is still running in this worktree.".to_string(),
                        )));
                    }

                    // F1 + F2: verify common_dir, branch identity, and ignored file fingerprint inside the gate.
                    let target_repo = backend.open(&path)?;
                    if !expected_common_dir.as_os_str().is_empty()
                        && canonical_common_dir(target_repo.as_ref()) != expected_common_dir
                    {
                        return Err(Error::new(ErrorKind::Backend(
                            "Worktree repository identity (common_dir) changed.".to_string(),
                        )));
                    }
                    let snap = target_repo.finish_snapshot(&CancellationToken::new())?;
                    if snap.detached || snap.branch.as_deref() != Some(expected_branch.as_str()) {
                        return Err(Error::new(ErrorKind::Backend(
                            "Worktree branch changed or detached before removal.".to_string(),
                        )));
                    }
                    match (&expected_ignored_fingerprint, &snap.ignored_fingerprint) {
                        (
                            Some(IgnoredFilesFingerprint::Verified {
                                entry_count: exp_c,
                                digest: exp_d,
                            }),
                            Some(IgnoredFilesFingerprint::Verified {
                                entry_count: cur_c,
                                digest: cur_d,
                            }),
                        ) if exp_c == cur_c && exp_d == cur_d => {}
                        _ => {
                            return Err(Error::new(ErrorKind::Backend(
                                "Git-ignored files changed or could not be verified before removal."
                                    .to_string(),
                            )));
                        }
                    }

                    // Never uses `--force` and never deletes branches.
                    origin_repo.remove_worktree_checked(&path, &expected_head)
                })();
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::WorktreeRemoved {
                        run_id,
                        item,
                        result,
                    },
                );
            });
        }
        WorktreeFinishEffect::CleanupTempWorktree {
            run_id,
            origin_repo_id,
            temp_path,
        } => {
            let Some(origin_repo) = repos.repository(&origin_repo_id).cloned() else {
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::TempWorktreeCleaned {
                        run_id,
                        removed: false,
                        note: Some(format!(
                            "Temporary worktree kept at {} (origin repository closed).",
                            temp_path.display()
                        )),
                    },
                );
                return;
            };
            let backend = Arc::clone(backend);
            executor.spawn(move || {
                let (removed, note) = (|| {
                    let gate = super::super::mutation_gate::for_repo(origin_repo.as_ref());
                    let _write = gate.write().unwrap_or_else(|e| e.into_inner());
                    let Ok(_lease) = origin_repo.acquire_operation_lease() else {
                        return (
                            false,
                            Some(format!(
                                "Temporary worktree kept at {} (repository busy).",
                                temp_path.display()
                            )),
                        );
                    };
                    let Ok(temp_repo) = backend.open(&temp_path) else {
                        return (true, None);
                    };
                    let Ok(snap) = temp_repo.finish_snapshot(&CancellationToken::new()) else {
                        return (
                            false,
                            Some(format!(
                                "Temporary worktree kept at {} (could not verify state).",
                                temp_path.display()
                            )),
                        );
                    };
                    if !snap.worktree_exists {
                        return (true, None);
                    }
                    if !snap.is_completely_clean()
                        || snap.operation != FinishOperation::None
                        || snap.has_squash_msg
                    {
                        return (
                            false,
                            Some(format!(
                                "Temporary target worktree kept at {} because it has unresolved conflicts or uncommitted changes.",
                                temp_path.display()
                            )),
                        );
                    }
                    match origin_repo.remove_worktree_with_output(&temp_path) {
                        Ok(_) => (true, None),
                        Err(err) => (
                            false,
                            Some(format!(
                                "Temporary worktree kept at {}: {err}",
                                temp_path.display()
                            )),
                        ),
                    }
                })();
                send_finish(
                    &msg_tx,
                    WorktreeFinishInternalMsg::TempWorktreeCleaned {
                        run_id,
                        removed,
                        note,
                    },
                );
            });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_load_preview(
    run_id: WorktreeFinishRunId,
    origin_repo_id: RepoId,
    start_index: usize,
    origin_repo: Arc<dyn GitRepository>,
    open_tabs: Vec<(RepoId, PathBuf, Arc<dyn GitRepository>)>,
    backend: Arc<dyn GitBackend>,
    dispatcher: Arc<dyn AgentDispatcher>,
    target_branch: String,
    mode: FinishMode,
    sources: Vec<FinishSourceRequest>,
) -> WorktreeFinishInternalMsg {
    let token = CancellationToken::new();
    let origin_common = canonical_common_dir(origin_repo.as_ref());

    let target_head = origin_repo
        .resolve_ref_commit(&format!("refs/heads/{target_branch}"))
        .ok()
        .flatten()
        .or_else(|| {
            origin_repo
                .resolve_ref_commit(&target_branch)
                .ok()
                .flatten()
        });

    let (target_host, target_snapshot) = match mode {
        FinishMode::RebaseOnto => (TargetHost::NotNeeded, None),
        FinishMode::MergeInto => resolve_target_host(
            run_id,
            origin_repo_id,
            &origin_repo,
            &open_tabs,
            &backend,
            &target_branch,
            &token,
        ),
    };

    let orca_available = matches!(
        dispatcher.availability(),
        AgentDispatchAvailability::Available { .. }
    );

    let mut source_results = Vec::with_capacity(sources.len());
    let mut top_error: Option<String> = None;

    for req in sources {
        let site_repo = match resolve_site_repo(&origin_repo, &open_tabs, &backend, &req.path) {
            Ok(repo) => repo,
            Err(err) => {
                if top_error.is_none() {
                    top_error = Some(err.to_string());
                }
                source_results.push(SourcePreviewResult {
                    path: req.path,
                    branch: req.branch,
                    locked_head: None,
                    snapshot: None,
                    active_sessions: Vec::new(),
                    preview: Err(err),
                    default_squash_message: None,
                });
                continue;
            }
        };

        // T-X4: enforce shared common_dir across all sources and origin.
        let src_common = canonical_common_dir(site_repo.as_ref());
        if src_common != origin_common {
            let err = Error::new(ErrorKind::Backend(format!(
                "Worktree '{}' belongs to a different repository.",
                req.path.display()
            )));
            if top_error.is_none() {
                top_error = Some(err.to_string());
            }
            source_results.push(SourcePreviewResult {
                path: req.path,
                branch: req.branch,
                locked_head: None,
                snapshot: None,
                active_sessions: Vec::new(),
                preview: Err(err),
                default_squash_message: None,
            });
            continue;
        }

        let snapshot = site_repo.finish_snapshot(&token).ok();
        let locked_head = snapshot.as_ref().and_then(|s| s.head.clone());
        let active_sessions = if orca_available {
            dispatcher
                .sessions(&req.path, &token)
                .unwrap_or_default()
                .into_iter()
                .filter(|s| s.state.is_active())
                .collect()
        } else {
            Vec::new()
        };

        let preview = match (&target_head, &locked_head) {
            (Some(t), Some(s)) => origin_repo.integration_preview(t, s, &token),
            _ => Err(Error::new(ErrorKind::Backend(
                "Missing target or source commit for preview.".to_string(),
            ))),
        };

        let default_squash_message = preview.as_ref().ok().map(|p| {
            if mode == FinishMode::RebaseOnto
                && let (Some(t), Some(s)) = (&target_head, &locked_head)
                && let Ok(Some((oldest, _))) = site_repo.linear_range_after(t, s)
                && let Ok(msg) = site_repo.squash_message_preview(&oldest, s)
                && !msg.trim().is_empty()
            {
                return msg;
            }
            default_squash_merge_message(&req.branch, &target_branch, &p.commit_subjects)
        });

        source_results.push(SourcePreviewResult {
            path: req.path,
            branch: req.branch,
            locked_head,
            snapshot,
            active_sessions,
            preview,
            default_squash_message,
        });
    }

    WorktreeFinishInternalMsg::PreviewLoaded {
        run_id,
        start_index,
        common_dir: origin_common,
        target_head,
        target_host,
        target_snapshot,
        sources: source_results,
        error: top_error,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_preflight_item(
    run_id: WorktreeFinishRunId,
    origin_repo: Arc<dyn GitRepository>,
    open_tabs: Vec<(RepoId, PathBuf, Arc<dyn GitRepository>)>,
    backend: Arc<dyn GitBackend>,
    item: usize,
    expected_common_dir: PathBuf,
    mode: FinishMode,
    target_branch: String,
    target_host: TargetHost,
    source_path: PathBuf,
    _source_branch: String,
    locked_source_head: CommitId,
    compute_expected_tree: bool,
) -> WorktreeFinishInternalMsg {
    let token = CancellationToken::new();
    let origin_common = canonical_common_dir(origin_repo.as_ref());
    if !expected_common_dir.as_os_str().is_empty() && origin_common != expected_common_dir {
        return WorktreeFinishInternalMsg::ItemPreflightLoaded {
            run_id,
            item,
            target_head: None,
            target_snapshot: None,
            source_head: None,
            source_snapshot: None,
            expected_merged_tree: None,
            error: Some("Origin repository identity (common_dir) changed.".to_string()),
        };
    }

    let target_head = origin_repo
        .resolve_ref_commit(&format!("refs/heads/{target_branch}"))
        .ok()
        .flatten()
        .or_else(|| {
            origin_repo
                .resolve_ref_commit(&target_branch)
                .ok()
                .flatten()
        });

    let mut error: Option<String> = None;
    let target_snapshot = if mode == FinishMode::MergeInto {
        match target_host.path() {
            Some(p) => match resolve_site_repo(&origin_repo, &open_tabs, &backend, p) {
                Ok(repo) => {
                    if !expected_common_dir.as_os_str().is_empty()
                        && canonical_common_dir(repo.as_ref()) != expected_common_dir
                    {
                        error = Some(format!(
                            "Target worktree '{}' no longer belongs to the same repository.",
                            p.display()
                        ));
                        None
                    } else {
                        match repo.finish_snapshot(&token) {
                            Ok(snap) => Some(snap),
                            Err(err) => {
                                error = Some(format!(
                                    "Could not inspect target worktree '{}': {err}",
                                    p.display()
                                ));
                                None
                            }
                        }
                    }
                }
                Err(err) => {
                    error = Some(format!(
                        "Could not open target worktree '{}': {err}",
                        p.display()
                    ));
                    None
                }
            },
            None => {
                error = Some("Missing target execution worktree for MergeInto.".to_string());
                None
            }
        }
    } else {
        None
    };

    let (source_head, source_snapshot) =
        match resolve_site_repo(&origin_repo, &open_tabs, &backend, &source_path) {
            Ok(repo) => {
                if !expected_common_dir.as_os_str().is_empty()
                    && canonical_common_dir(repo.as_ref()) != expected_common_dir
                {
                    if error.is_none() {
                        error = Some(format!(
                            "Source worktree '{}' no longer belongs to the same repository.",
                            source_path.display()
                        ));
                    }
                    (None, None)
                } else {
                    match repo.finish_snapshot(&token) {
                        Ok(snap) => (snap.head.clone(), Some(snap)),
                        Err(err) => {
                            if error.is_none() {
                                error = Some(err.to_string());
                            }
                            (None, None)
                        }
                    }
                }
            }
            Err(err) => {
                if error.is_none() {
                    error = Some(err.to_string());
                }
                (None, None)
            }
        };

    let expected_merged_tree = if compute_expected_tree
        && error.is_none()
        && let Some(t) = &target_head
    {
        origin_repo
            .integration_preview(t, &locked_source_head, &token)
            .ok()
            .and_then(|p| p.merged_tree)
    } else {
        None
    };

    WorktreeFinishInternalMsg::ItemPreflightLoaded {
        run_id,
        item,
        target_head,
        target_snapshot,
        source_head,
        source_snapshot,
        expected_merged_tree,
        error,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_step(
    run_id: WorktreeFinishRunId,
    origin_repo: Arc<dyn GitRepository>,
    open_tabs: Vec<(RepoId, PathBuf, Arc<dyn GitRepository>)>,
    backend: Arc<dyn GitBackend>,
    item: usize,
    step: FinishStep,
    site: PathBuf,
    guard: StepSiteGuard,
    action: FinishStepAction,
    verify_rebase_target: Option<CommitId>,
    auth: Option<StagedGitAuth>,
) -> WorktreeFinishInternalMsg {
    let token = CancellationToken::new();
    let site_repo = match resolve_site_repo(&origin_repo, &open_tabs, &backend, &site) {
        Ok(repo) => repo,
        Err(err) => {
            return WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item,
                step,
                before: Default::default(),
                after: None,
                command: Err(err),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            };
        }
    };

    let gate = super::super::mutation_gate::for_repo(site_repo.as_ref());
    let _write = gate.write().unwrap_or_else(|e| e.into_inner());
    let _lease = match site_repo.acquire_operation_lease() {
        Ok(lease) => lease,
        Err(err) => {
            return WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item,
                step,
                before: Default::default(),
                after: None,
                command: Err(err),
                precheck_violation: None,
                rebased_range: None,
                target_is_ancestor: None,
            };
        }
    };

    // F1: verify live on-disk common_dir (not only a cached tab handle) and snapshot inside the
    // mutation gate before any Git mutation.
    if !guard.expected_common_dir.as_os_str().is_empty() {
        let live_common_matches =
            backend.open(&site).ok().is_some_and(|fresh| {
                canonical_common_dir(fresh.as_ref()) == guard.expected_common_dir
            }) && canonical_common_dir(site_repo.as_ref()) == guard.expected_common_dir;
        if !live_common_matches {
            let msg = format!(
                "Execution worktree '{}' no longer matches the expected repository.",
                site.display()
            );
            return WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item,
                step,
                before: Default::default(),
                after: None,
                command: Err(Error::new(ErrorKind::Backend(msg.clone()))),
                precheck_violation: Some(msg),
                rebased_range: None,
                target_is_ancestor: None,
            };
        }
    }

    let before = match site_repo.finish_snapshot(&token) {
        Ok(snap) => snap,
        Err(err) => {
            let msg = format!("Failed to verify worktree snapshot before executing step: {err}");
            return WorktreeFinishInternalMsg::StepFinished {
                run_id,
                item,
                step,
                before: Default::default(),
                after: None,
                command: Err(err),
                precheck_violation: Some(msg),
                rebased_range: None,
                target_is_ancestor: None,
            };
        }
    };

    if let Some(violation) = check_step_site_guard(&before, &guard) {
        return WorktreeFinishInternalMsg::StepFinished {
            run_id,
            item,
            step,
            before: before.clone(),
            after: Some(before),
            command: Err(Error::new(ErrorKind::Backend(violation.clone()))),
            precheck_violation: Some(violation),
            rebased_range: None,
            target_is_ancestor: None,
        };
    }

    let _scoped_auth = auth.map(ScopedStagedGitAuth::stage);

    let command: Result<FinishStepCommandOutput, Error> = match action {
        FinishStepAction::Merge {
            source_head,
            message,
        } => site_repo.merge_commit_at(&source_head, &message),
        FinishStepAction::SquashMerge { source_head } => site_repo.squash_merge_at(&source_head),
        FinishStepAction::SquashCommit { message } => site_repo
            .conclude_merge(Some(&message))
            .map(FinishStepCommandOutput::from_output),
        FinishStepAction::ConcludeMerge { message } => site_repo
            .conclude_merge(message.as_deref())
            .map(FinishStepCommandOutput::from_output),
        FinishStepAction::Rebase { target_head } => site_repo.rebase_onto_at(&target_head),
        FinishStepAction::RebaseContinue { .. } => site_repo.rebase_continue_step(),
        FinishStepAction::SquashRange {
            oldest,
            expected_head,
            message,
        } => site_repo
            .squash_commits_with_output(&oldest, &expected_head, &message)
            .map(FinishStepCommandOutput::from_output),
        FinishStepAction::AbortMerge { safety } => {
            match evaluate_abort_safety(&before, &safety, false) {
                // B1: the backend still refuses when restoring HEAD would delete ignored or
                // untracked data that the status-based verdict above cannot see.
                AbortSafetyVerdict::SafeFullAbort => site_repo
                    .abort_merge_checked()
                    .map(FinishStepCommandOutput::from_output),
                AbortSafetyVerdict::Refused(reason) => Err(Error::new(ErrorKind::Backend(reason))),
                AbortSafetyVerdict::SafeScopedSquash { .. } => Err(Error::new(ErrorKind::Backend(
                    "Unexpected scoped abort verdict for merge.".to_string(),
                ))),
            }
        }
        FinishStepAction::AbortSquashMerge { safety } => {
            match evaluate_abort_safety(&before, &safety, true) {
                AbortSafetyVerdict::SafeScopedSquash { owned_paths } => site_repo
                    .abort_squash_merge_paths(&owned_paths)
                    .map(FinishStepCommandOutput::from_output),
                AbortSafetyVerdict::SafeFullAbort => Err(Error::new(ErrorKind::Backend(
                    "Unexpected un-scoped abort verdict for squash merge.".to_string(),
                ))),
                AbortSafetyVerdict::Refused(reason) => Err(Error::new(ErrorKind::Backend(reason))),
            }
        }
        FinishStepAction::AbortRebase { safety } => {
            match evaluate_abort_safety(&before, &safety, false) {
                AbortSafetyVerdict::SafeFullAbort => site_repo
                    .abort_rebase_checked()
                    .map(FinishStepCommandOutput::from_output),
                AbortSafetyVerdict::Refused(reason) => Err(Error::new(ErrorKind::Backend(reason))),
                AbortSafetyVerdict::SafeScopedSquash { .. } => Err(Error::new(ErrorKind::Backend(
                    "Unexpected scoped abort verdict for rebase.".to_string(),
                ))),
            }
        }
        FinishStepAction::RollbackToHead { head } => site_repo
            .reset_with_output(head.as_ref(), ResetMode::Hard)
            .map(FinishStepCommandOutput::from_output),
    };

    let after = site_repo.finish_snapshot(&token).ok();
    let (target_is_ancestor, rebased_range) = match (&after, &verify_rebase_target) {
        (Some(snap), Some(target)) if snap.operation == FinishOperation::None => {
            if let Some(head) = &snap.head {
                let is_anc = site_repo.is_commit_ancestor(target, head).ok();
                let range = if is_anc == Some(true) {
                    site_repo.linear_range_after(target, head).ok().flatten()
                } else {
                    None
                };
                (is_anc, range)
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    };

    WorktreeFinishInternalMsg::StepFinished {
        run_id,
        item,
        step,
        before,
        after,
        command,
        precheck_violation: None,
        rebased_range,
        target_is_ancestor,
    }
}

fn check_step_site_guard(before: &FinishSnapshot, guard: &StepSiteGuard) -> Option<String> {
    if !before.worktree_exists {
        return Some("Execution worktree no longer exists.".to_string());
    }
    if before.is_bare {
        return Some("Execution worktree is bare.".to_string());
    }
    if guard.allow_detached_rebase {
        if before.operation != FinishOperation::Rebase {
            return Some(format!(
                "Expected an in-progress rebase at execution site, found {:?}.",
                before.operation
            ));
        }
    } else if !guard.expected_branch.is_empty()
        && (before.detached || before.branch.as_deref() != Some(guard.expected_branch.as_str()))
    {
        return Some(format!(
            "Execution worktree switched away from '{}' (found {:?}, detached={}).",
            guard.expected_branch, before.branch, before.detached
        ));
    }
    if let Some(expected_head) = &guard.expected_head
        && before.head.as_ref() != Some(expected_head)
    {
        return Some(format!(
            "Execution worktree HEAD changed before step (expected {}, found {:?}).",
            expected_head.as_ref(),
            before.head.as_ref().map(|h| h.as_ref())
        ));
    }
    if guard.require_no_operation
        && (before.operation != FinishOperation::None || before.has_squash_msg)
    {
        return Some("Execution worktree already has an in-progress Git operation.".to_string());
    }
    if guard.require_clean_tracked && !before.is_clean_tracked() {
        return Some("Execution worktree has uncommitted tracked changes.".to_string());
    }
    None
}

enum AbortSafetyVerdict {
    SafeFullAbort,
    SafeScopedSquash { owned_paths: Vec<PathBuf> },
    Refused(String),
}

fn is_safe_literal_repo_file_path(path: &Path) -> bool {
    let raw = path.as_os_str().as_encoded_bytes();
    if raw.is_empty() || path.is_absolute() || raw.ends_with(b"/") || raw.ends_with(b"\\") {
        return false;
    }
    let mut has_normal = false;
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => has_normal = true,
            _ => return false,
        }
    }
    has_normal
}

fn paths_overlap(a: &Path, b: &Path) -> bool {
    a == b || a.starts_with(b) || b.starts_with(a)
}

fn evaluate_abort_safety(
    before: &FinishSnapshot,
    safety: &AbortSafetyBaseline,
    is_squash: bool,
) -> AbortSafetyVerdict {
    let Some(expected_head) = &safety.expected_head else {
        return AbortSafetyVerdict::Refused(
            "Expected pre-conflict HEAD is missing; preserving worktree and index for manual recovery."
                .to_string(),
        );
    };
    if before.head.as_ref() != Some(expected_head) {
        return AbortSafetyVerdict::Refused(
            "HEAD changed since the conflict paused; preserving worktree for manual recovery."
                .to_string(),
        );
    }
    let Some(baseline) = &safety.baseline_entries else {
        return AbortSafetyVerdict::Refused(
            "Could not verify the conflict baseline; preserving worktree and index for manual recovery."
                .to_string(),
        );
    };
    let Some(current) = &before.dirty_entries else {
        return AbortSafetyVerdict::Refused(
            "Could not verify current working tree and index entries; preserving site for manual recovery."
                .to_string(),
        );
    };
    if before.untracked_count > before.untracked_paths.len() {
        return AbortSafetyVerdict::Refused(
            "Too many untracked files to guarantee safe abort; preserving worktree for manual recovery."
                .to_string(),
        );
    }

    let mut baseline_map: HashMap<(PathBuf, Option<PathBuf>), &WorktreeEntryDigest> =
        HashMap::with_capacity(baseline.len());
    let mut baseline_paths: HashSet<PathBuf> = HashSet::with_capacity(baseline.len());
    for entry in baseline {
        baseline_map.insert((entry.path.clone(), entry.orig_path.clone()), entry);
        baseline_paths.insert(entry.path.clone());
        if let Some(orig) = &entry.orig_path {
            baseline_paths.insert(orig.clone());
        }
    }

    if is_squash {
        if baseline_paths.is_empty() {
            return AbortSafetyVerdict::Refused(
                "Squash conflict ownership baseline is empty; preserving index and worktree."
                    .to_string(),
            );
        }
        for owned in &baseline_paths {
            if !is_safe_literal_repo_file_path(owned) {
                return AbortSafetyVerdict::Refused(format!(
                    "Uncertain literal file path '{}' in squash baseline; preserving index and worktree.",
                    owned.display()
                ));
            }
        }
    }

    let mut current_map: HashMap<(PathBuf, Option<PathBuf>), &WorktreeEntryDigest> =
        HashMap::with_capacity(current.len());
    for entry in current {
        current_map.insert((entry.path.clone(), entry.orig_path.clone()), entry);
    }

    for (key, expected_entry) in &baseline_map {
        match current_map.get(key) {
            Some(cur_entry) if *cur_entry == *expected_entry => {}
            _ => {
                return AbortSafetyVerdict::Refused(format!(
                    "Post-conflict changes detected in '{}'; preserving working tree and index for manual recovery.",
                    expected_entry.path.display()
                ));
            }
        }
    }

    for untracked in &before.untracked_paths {
        if baseline_paths
            .iter()
            .any(|owned| paths_overlap(untracked, owned))
        {
            return AbortSafetyVerdict::Refused(format!(
                "Untracked file '{}' collides with a conflict path; preserving worktree for manual recovery.",
                untracked.display()
            ));
        }
    }

    let external_entries: Vec<&WorktreeEntryDigest> = current
        .iter()
        .filter(|entry| !baseline_map.contains_key(&(entry.path.clone(), entry.orig_path.clone())))
        .collect();

    if is_squash {
        for ext in &external_entries {
            if ext.status_code.starts_with("u ") {
                return AbortSafetyVerdict::Refused(format!(
                    "Unowned unmerged conflict entry '{}' detected; preserving index and worktree.",
                    ext.path.display()
                ));
            }
            let overlaps_owned = baseline_paths.iter().any(|owned| {
                paths_overlap(&ext.path, owned)
                    || ext
                        .orig_path
                        .as_ref()
                        .is_some_and(|orig| paths_overlap(orig, owned))
            });
            if overlaps_owned {
                return AbortSafetyVerdict::Refused(format!(
                    "Post-conflict change '{}' overlaps a squash-owned path; preserving index and worktree.",
                    ext.path.display()
                ));
            }
        }
        let mut owned_paths: Vec<PathBuf> = baseline_paths.into_iter().collect();
        owned_paths.sort();
        AbortSafetyVerdict::SafeScopedSquash { owned_paths }
    } else if let Some(ext) = external_entries.first() {
        AbortSafetyVerdict::Refused(format!(
            "Post-conflict staged or modified user file '{}' detected; refusing destructive abort to preserve user data.",
            ext.path.display()
        ))
    } else {
        AbortSafetyVerdict::SafeFullAbort
    }
}

fn resolve_target_host(
    run_id: WorktreeFinishRunId,
    origin_repo_id: RepoId,
    origin_repo: &Arc<dyn GitRepository>,
    open_tabs: &[(RepoId, PathBuf, Arc<dyn GitRepository>)],
    backend: &Arc<dyn GitBackend>,
    target_branch: &str,
    token: &CancellationToken,
) -> (
    TargetHost,
    Option<gitcomet_core::worktree_finish::FinishSnapshot>,
) {
    // 1. Check if target_branch is checked out in the origin tab.
    if let Ok(origin_snap) = origin_repo.finish_snapshot(token)
        && origin_snap.branch.as_deref() == Some(target_branch)
    {
        return (
            TargetHost::OpenTab {
                repo_id: origin_repo_id,
                path: origin_repo.spec().workdir.clone(),
            },
            Some(origin_snap),
        );
    }

    // 2. Check if target_branch is checked out in another worktree of this repo.
    let other_path = origin_repo
        .branch_checked_out_in_other_worktree(target_branch)
        .ok()
        .flatten()
        .or_else(|| {
            origin_repo.list_worktrees(token).ok().and_then(|wts| {
                wts.into_iter()
                    .find(|wt| wt.branch.as_deref() == Some(target_branch))
                    .map(|wt| wt.path)
            })
        });

    if let Some(path) = other_path {
        let canonical = canonicalize_or_original(path.clone());
        if let Some((tab_id, tab_path, tab_repo)) = open_tabs
            .iter()
            .find(|(_, p, _)| canonicalize_or_original(p.clone()) == canonical)
        {
            let snap = tab_repo.finish_snapshot(token).ok();
            return (
                TargetHost::OpenTab {
                    repo_id: *tab_id,
                    path: tab_path.clone(),
                },
                snap,
            );
        }
        let snap = backend
            .open(&path)
            .ok()
            .and_then(|r| r.finish_snapshot(token).ok());
        return (TargetHost::LinkedWorktree { path }, snap);
    }

    // 3. D1: target branch is not checked out anywhere -> plan a dedicated temporary worktree
    // under the product's per-user state dir (never inside `.git`), created only after Confirm.
    let temp_path = temporary_worktree_path(run_id, target_branch);
    (
        TargetHost::Temporary {
            path: temp_path,
            created: false,
        },
        None,
    )
}

fn temporary_worktree_path(run_id: WorktreeFinishRunId, branch: &str) -> PathBuf {
    let base = gitcomet_base::platform::dirs::state_dir().unwrap_or_else(|| {
        std::env::temp_dir().join(gitcomet_core::identity::current().directory_name())
    });
    let sanitized: String = branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect();
    base.join("worktree-finish-temp")
        .join(format!("run-{}-{}", run_id.0, sanitized))
}

fn snapshot_open_tabs(
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &RepositoryRuntimes,
) -> Vec<(RepoId, PathBuf, Arc<dyn GitRepository>)> {
    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
    state
        .repos
        .iter()
        .filter_map(|repo| {
            let handle = repos.repository(&repo.id).cloned()?;
            Some((repo.id, repo.spec.workdir.clone(), handle))
        })
        .collect()
}

fn resolve_site_repo(
    origin_repo: &Arc<dyn GitRepository>,
    open_tabs: &[(RepoId, PathBuf, Arc<dyn GitRepository>)],
    backend: &Arc<dyn GitBackend>,
    site: &Path,
) -> Result<Arc<dyn GitRepository>, Error> {
    let canonical_site = canonicalize_or_original(site.to_path_buf());
    if canonicalize_or_original(origin_repo.spec().workdir.clone()) == canonical_site {
        return Ok(Arc::clone(origin_repo));
    }
    if let Some((_, _, repo)) = open_tabs
        .iter()
        .find(|(_, p, _)| canonicalize_or_original(p.clone()) == canonical_site)
    {
        return Ok(Arc::clone(repo));
    }
    backend.open(site)
}

fn canonical_common_dir(repo: &dyn GitRepository) -> PathBuf {
    canonicalize_or_original(
        repo.common_dir()
            .unwrap_or_else(|| repo.spec().workdir.clone()),
    )
}

fn send_finish(msg_tx: &StoreWorkerSender, msg: WorktreeFinishInternalMsg) {
    send_or_log(
        msg_tx,
        Msg::Internal(InternalMsg::WorktreeFinish(Box::new(msg))),
    );
}
