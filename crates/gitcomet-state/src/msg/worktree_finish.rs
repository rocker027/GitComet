//! Messages and effects for the Worktree Finish workflow.

use crate::model::{
    DriftResolution, FinishMode, FinishSourceRequest, FinishStep, RepoId, TargetHost,
    WorktreeFinishRunId,
};
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::Error;
use gitcomet_core::services::CommandOutput;
use gitcomet_core::worktree_finish::{
    AgentDispatchAvailability, AgentLaunch, AgentSession, FinishSnapshot, FinishStepCommandOutput,
    IgnoredFilesFingerprint, InstalledAgent, IntegrationPreview, TreeId, WorktreeEntryDigest,
};
use std::path::PathBuf;

/// User-facing messages for the Worktree Finish workflow.
#[derive(Clone, Debug)]
pub enum WorktreeFinishMsg {
    /// Start or refresh a preview run for the given repository, target branch, mode, and ordered sources.
    Preview {
        repo_id: RepoId,
        target_branch: String,
        mode: FinishMode,
        sources: Vec<FinishSourceRequest>,
        /// Paths currently hosting a running embedded GitComet terminal (supplied by UI).
        running_terminal_paths: Vec<PathBuf>,
    },
    /// Explicitly confirm a previewed run. Rejected if `fingerprint` does not match the current preview.
    Confirm {
        run_id: WorktreeFinishRunId,
        fingerprint: u64,
        running_terminal_paths: Vec<PathBuf>,
    },
    /// Re-snapshot the currently paused worktree and classify state/drift (§5.4).
    Recheck { run_id: WorktreeFinishRunId },
    /// Proceed after conflict resolution (conclude merge, squash commit, or `rebase --continue`).
    Continue { run_id: WorktreeFinishRunId },
    /// Revert the current item's conflicted/in-progress state and skip it.
    SkipItem { run_id: WorktreeFinishRunId },
    /// Stop processing remaining queued items; optionally keep or revert current conflict site,
    /// then run batch-end cleanup on already verified completed items.
    AbortRemaining {
        run_id: WorktreeFinishRunId,
        keep_conflict_state: bool,
    },
    /// Query Orca CLI availability and dynamically installed agents for the conflict site.
    LoadAgents { run_id: WorktreeFinishRunId },
    /// Launch a dynamically discovered Orca agent in the exact conflict site worktree.
    DispatchAgent {
        run_id: WorktreeFinishRunId,
        agent_id: String,
    },
    /// Poll Orca session status for a dispatched agent (or trigger timeout check).
    PollAgent { run_id: WorktreeFinishRunId },
    /// Switch an active `AgentDispatched` item back to manual resolution without stopping the agent.
    SwitchToManual { run_id: WorktreeFinishRunId },
    /// Answer a `NeedsDecision` drift prompt (§5.4).
    ResolveDrift {
        run_id: WorktreeFinishRunId,
        choice: DriftResolution,
    },
    /// Retry a credential/signing-blocked step after credentials have been staged.
    RetryCredential { run_id: WorktreeFinishRunId },
    /// Remove a finished or aborted run from `AppState`.
    Dismiss { run_id: WorktreeFinishRunId },
    /// Update the set of worktree paths that currently have an active embedded GitComet terminal.
    UpdateRunningTerminals {
        run_id: WorktreeFinishRunId,
        paths: Vec<PathBuf>,
    },
    /// Synchronize the live set of embedded GitComet terminal paths globally and across active runs (F9).
    SyncRunningTerminals { paths: Vec<PathBuf> },
}

/// Site identity and state assertions verified inside the mutation gate before running a Git step (F1).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StepSiteGuard {
    pub expected_common_dir: PathBuf,
    pub expected_branch: String,
    pub expected_head: Option<CommitId>,
    /// True during `RebaseContinue` / `AbortRebase`, where Git detaches HEAD while rebasing.
    pub allow_detached_rebase: bool,
    pub require_clean_tracked: bool,
    pub require_no_operation: bool,
}

/// Baseline conflict entries verified before Skip/Abort so post-conflict user data is preserved (F4).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AbortSafetyBaseline {
    pub expected_head: Option<CommitId>,
    pub baseline_entries: Option<Vec<WorktreeEntryDigest>>,
}

/// Preview payload gathered for a single source worktree.
#[derive(Clone, Debug)]
pub struct SourcePreviewResult {
    pub path: PathBuf,
    pub branch: String,
    pub locked_head: Option<CommitId>,
    pub snapshot: Option<FinishSnapshot>,
    pub active_sessions: Vec<AgentSession>,
    pub preview: Result<IntegrationPreview, Error>,
    pub default_squash_message: Option<String>,
}

/// Result of the batch-end pre-cleanup re-check for one source worktree.
#[derive(Clone, Debug)]
pub struct SourceCleanupCheckResult {
    pub index: usize,
    pub snapshot: Result<FinishSnapshot, Error>,
    pub active_sessions: Result<Vec<AgentSession>, AgentCheckStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentCheckStatus {
    OrcaNotFound,
    QueryFailed(String),
}

/// Worker-to-reducer completion messages for the Worktree Finish workflow.
#[derive(Clone, Debug)]
pub enum WorktreeFinishInternalMsg {
    PreviewLoaded {
        run_id: WorktreeFinishRunId,
        start_index: usize,
        common_dir: PathBuf,
        target_head: Option<CommitId>,
        target_host: TargetHost,
        target_snapshot: Option<FinishSnapshot>,
        sources: Vec<SourcePreviewResult>,
        error: Option<String>,
    },
    TempWorktreePrepared {
        run_id: WorktreeFinishRunId,
        path: PathBuf,
        result: Result<FinishSnapshot, Error>,
    },
    ItemPreflightLoaded {
        run_id: WorktreeFinishRunId,
        item: usize,
        target_head: Option<CommitId>,
        target_snapshot: Option<FinishSnapshot>,
        source_head: Option<CommitId>,
        source_snapshot: Option<FinishSnapshot>,
        expected_merged_tree: Option<TreeId>,
        error: Option<String>,
    },
    StepFinished {
        run_id: WorktreeFinishRunId,
        item: usize,
        step: FinishStep,
        before: FinishSnapshot,
        after: Option<FinishSnapshot>,
        command: Result<FinishStepCommandOutput, Error>,
        /// Set when the in-gate pre-execution identity/state guard refused before running Git (F1).
        precheck_violation: Option<String>,
        /// For `Rebase`: linear commit count after target (`None` if not applicable).
        rebased_range: Option<(CommitId, usize)>,
        /// For `Rebase`: whether `target_head` is an ancestor of post-step HEAD.
        target_is_ancestor: Option<bool>,
    },
    RecheckLoaded {
        run_id: WorktreeFinishRunId,
        item: usize,
        snapshot: Result<FinishSnapshot, Error>,
        target_is_ancestor: Option<bool>,
        rebased_range: Option<(CommitId, usize)>,
    },
    AgentsLoaded {
        run_id: WorktreeFinishRunId,
        availability: AgentDispatchAvailability,
        agents: Result<Vec<InstalledAgent>, Error>,
    },
    AgentLaunched {
        run_id: WorktreeFinishRunId,
        item: usize,
        agent_id: String,
        request_id: u64,
        result: Result<AgentLaunch, Error>,
    },
    AgentPolled {
        run_id: WorktreeFinishRunId,
        item: usize,
        sessions: Result<Vec<AgentSession>, Error>,
    },
    CleanupChecked {
        run_id: WorktreeFinishRunId,
        results: Vec<SourceCleanupCheckResult>,
        orca_unavailable: bool,
        live_terminal_paths: Vec<PathBuf>,
    },
    WorktreeRemoved {
        run_id: WorktreeFinishRunId,
        item: usize,
        result: Result<CommandOutput, Error>,
    },
    TempWorktreeCleaned {
        run_id: WorktreeFinishRunId,
        removed: bool,
        note: Option<String>,
    },
}

/// Concrete step action dispatched to the background worker.
#[derive(Clone, Debug)]
pub enum FinishStepAction {
    Merge {
        source_head: CommitId,
        message: String,
    },
    SquashMerge {
        source_head: CommitId,
    },
    SquashCommit {
        message: String,
    },
    Rebase {
        target_head: CommitId,
    },
    RebaseContinue {
        target_head: CommitId,
    },
    SquashRange {
        oldest: CommitId,
        expected_head: CommitId,
        message: String,
    },
    ConcludeMerge {
        message: Option<String>,
    },
    AbortMerge {
        safety: AbortSafetyBaseline,
    },
    AbortSquashMerge {
        safety: AbortSafetyBaseline,
    },
    AbortRebase {
        safety: AbortSafetyBaseline,
    },
    RollbackToHead {
        head: CommitId,
    },
}

/// Side effects emitted by the Worktree Finish reducer.
#[derive(Clone, Debug)]
pub enum WorktreeFinishEffect {
    LoadPreview {
        run_id: WorktreeFinishRunId,
        origin_repo_id: RepoId,
        start_index: usize,
        target_branch: String,
        mode: FinishMode,
        sources: Vec<FinishSourceRequest>,
    },
    PrepareTempWorktree {
        run_id: WorktreeFinishRunId,
        origin_repo_id: RepoId,
        target_branch: String,
        temp_path: PathBuf,
    },
    PreflightItem {
        run_id: WorktreeFinishRunId,
        origin_repo_id: RepoId,
        item: usize,
        expected_common_dir: PathBuf,
        mode: FinishMode,
        target_branch: String,
        target_host: TargetHost,
        source_path: PathBuf,
        source_branch: String,
        locked_source_head: CommitId,
        compute_expected_tree: bool,
    },
    RunStep {
        run_id: WorktreeFinishRunId,
        origin_repo_id: RepoId,
        item: usize,
        step: FinishStep,
        site: PathBuf,
        guard: StepSiteGuard,
        action: FinishStepAction,
        /// Optional target commit for verifying rebase ancestor post-step.
        verify_rebase_target: Option<CommitId>,
        auth: Option<StagedGitAuth>,
    },
    RecheckSite {
        run_id: WorktreeFinishRunId,
        origin_repo_id: RepoId,
        item: usize,
        site: PathBuf,
        verify_rebase_target: Option<CommitId>,
    },
    LoadAgents {
        run_id: WorktreeFinishRunId,
        site: PathBuf,
    },
    LaunchAgent {
        run_id: WorktreeFinishRunId,
        item: usize,
        site: PathBuf,
        agent_id: String,
        request_id: u64,
        prompt: String,
        retry_operation_id: Option<String>,
    },
    PollAgent {
        run_id: WorktreeFinishRunId,
        item: usize,
        site: PathBuf,
    },
    CheckCleanup {
        run_id: WorktreeFinishRunId,
        origin_repo_id: RepoId,
        items: Vec<(usize, PathBuf)>,
    },
    RemoveWorktree {
        run_id: WorktreeFinishRunId,
        origin_repo_id: RepoId,
        item: usize,
        path: PathBuf,
        expected_common_dir: PathBuf,
        expected_branch: String,
        expected_head: CommitId,
        expected_ignored_fingerprint: Option<IgnoredFilesFingerprint>,
    },
    CleanupTempWorktree {
        run_id: WorktreeFinishRunId,
        origin_repo_id: RepoId,
        temp_path: PathBuf,
    },
}

impl WorktreeFinishEffect {
    #[cfg(test)]
    pub(crate) fn git_auth(&self) -> Option<&StagedGitAuth> {
        match self {
            Self::RunStep { auth, .. } => auth.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn git_auth_slot(&mut self) -> Option<&mut Option<StagedGitAuth>> {
        match self {
            Self::RunStep { auth, .. } => Some(auth),
            _ => None,
        }
    }
}
