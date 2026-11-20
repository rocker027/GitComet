//! State model for the Worktree Finish (batch merge/rebase/squash/cleanup) workflow.

use super::{Loadable, RepoId};
use gitcomet_core::domain::CommitId;
use gitcomet_core::worktree_finish::{
    AgentDispatchAvailability, AgentSession, AgentSessionState, FinishSnapshot, InstalledAgent,
    IntegrationPreview, TreeId, WorktreeEntryDigest,
};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Unique monotonic identifier for a worktree finish run.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WorktreeFinishRunId(pub u64);

/// Integration mode for a finish run.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FinishMode {
    #[default]
    MergeInto,
    /// Pure Git rebase semantics: rewrites each source branch onto target without moving target.
    RebaseOnto,
}

/// Where MergeInto operations execute for the target branch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetHost {
    OpenTab {
        repo_id: RepoId,
        path: PathBuf,
    },
    LinkedWorktree {
        path: PathBuf,
    },
    /// Dedicated temporary worktree created only after user confirmation (D1).
    Temporary {
        path: PathBuf,
        created: bool,
    },
    /// RebaseOnto operates in each source worktree and never checks out target.
    NotNeeded,
}

impl TargetHost {
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::OpenTab { path, .. }
            | Self::LinkedWorktree { path }
            | Self::Temporary { path, .. } => Some(path.as_path()),
            Self::NotNeeded => None,
        }
    }
}

/// Target branch metadata for a finish run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishTarget {
    pub branch: String,
    pub initial_head: Option<CommitId>,
    pub locked_head: Option<CommitId>,
    pub host: TargetHost,
    pub snapshot: Option<FinishSnapshot>,
}

/// Per-source input submitted when initiating or refreshing a finish preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishSourceRequest {
    pub path: PathBuf,
    pub branch: String,
    pub squash: bool,
    pub remove_after: bool,
    pub squash_message: Option<String>,
}

impl FinishSourceRequest {
    pub fn new(path: PathBuf, branch: String) -> Self {
        Self {
            path,
            branch,
            squash: false,
            remove_after: false,
            squash_message: None,
        }
    }
}

/// Concrete execution step for an item in a worktree finish run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinishStep {
    PrepareTempWorktree,
    PreflightItem,
    Merge,
    SquashMerge,
    SquashCommit,
    Rebase,
    RebaseContinue,
    SquashRange,
    ConcludeMerge,
    AbortItem,
    RollbackToHead { head: CommitId },
}

/// Verification grade for a squash commit's tree contents (§3.4 & D9).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SquashContentCheck {
    /// Clean squash whose resulting tree matched the pre-step `merge-tree` tree.
    Verified,
    /// Conflict-resolved squash whose resolution was explicitly reviewed and continued by the user.
    ReviewedResolution,
}

/// Successful terminal outcome of a single source item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ItemOutcome {
    FastForwarded {
        new_head: CommitId,
    },
    Merged {
        commit: CommitId,
    },
    Squashed {
        commit: CommitId,
        content_verified: SquashContentCheck,
    },
    NothingNew,
    Rebased {
        new_head: CommitId,
        commits: usize,
    },
    RebasedSquashed {
        new_head: CommitId,
    },
}

impl ItemOutcome {
    pub fn result_commit(&self) -> Option<&CommitId> {
        match self {
            Self::FastForwarded { new_head }
            | Self::Rebased { new_head, .. }
            | Self::RebasedSquashed { new_head } => Some(new_head),
            Self::Merged { commit } | Self::Squashed { commit, .. } => Some(commit),
            Self::NothingNew => None,
        }
    }
}

/// Why an item was skipped rather than integrated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkipReason {
    UserSkipped,
    AbortedByUser,
    SourceMovedSkipped,
}

/// Unexpected or external state change requiring an explicit user decision (§5.4).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DriftKind {
    /// The operation finished externally (e.g. an agent or the user committed) with an expected commit shape.
    AdoptCommit {
        before_head: CommitId,
        after_head: CommitId,
        snapshot: FinishSnapshot,
    },
    /// The operation was aborted externally and HEAD returned to `before_head`.
    Aborted {
        before_head: CommitId,
        snapshot: FinishSnapshot,
    },
    /// Source branch moved after preview lock before execution began.
    SourceMoved {
        locked_head: CommitId,
        current_head: CommitId,
        snapshot: FinishSnapshot,
    },
    /// Target branch moved externally between items.
    TargetMoved {
        expected_head: CommitId,
        current_head: CommitId,
        snapshot: FinishSnapshot,
    },
    /// HEAD moved unexpectedly, branch switched, worktree disappeared, or unexpected dirty state appeared.
    Unexpected {
        summary: String,
        snapshot: FinishSnapshot,
    },
}

/// User decision answering a [`DriftKind`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DriftResolution {
    AdoptCommit,
    RollbackToBeforeHead,
    RequeueItem,
    SkipItem,
    /// Merge-only: proceed with the originally locked commit and cancel cleanup for this source.
    KeepLockedHead,
    RefreshPreview,
    AbortRemaining,
}

/// Specific reason a requested source worktree folder removal was conservatively refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupBlocker {
    PrimaryWorktree,
    BareWorktree,
    WorktreeMissing,
    HeadChangedAfterFinish,
    BranchChangedOrDetached,
    DirtyOrUntracked,
    InProgressOperation,
    WorktreeLocked,
    HasSubmodules,
    OpenGitCometTerminal,
    ActiveOrcaAgent,
    /// D4: an agent was dispatched or previously observed, and occupancy can no longer be verified.
    OrcaSessionUnverifiable,
    /// D3: ignored files changed since preview disclosure or were not disclosed.
    IgnoredFilesChangedOrUnverified,
    /// D9: conflict resolution or final state could not be verified.
    ResolutionUnverified,
    DisqualifiedByDrift(String),
    GitRefused(String),
}

/// Cleanup outcome recorded for a source at the end of the batch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CleanupOutcome {
    #[default]
    NotRequested,
    Pending,
    Removed,
    Kept(CleanupBlocker),
}

/// Per-item lifecycle state inside a [`WorktreeFinishRun`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ItemPhase {
    #[default]
    Queued,
    Running(FinishStep),
    PausedConflict {
        site: PathBuf,
        stops: u32,
        before_head: CommitId,
        snapshot: FinishSnapshot,
    },
    /// Launch request sent to the external agent dispatcher; blocks re-entrant launches (F5).
    AgentLaunching {
        site: PathBuf,
        stops: u32,
        before_head: CommitId,
        agent_id: String,
        request_id: u64,
        started_at: SystemTime,
        snapshot: FinishSnapshot,
    },
    AgentDispatched {
        site: PathBuf,
        stops: u32,
        before_head: CommitId,
        agent_id: String,
        session_id: Option<String>,
        operation_id: Option<String>,
        launched_at: SystemTime,
        last_polled_at: Option<SystemTime>,
        agent_state: Option<AgentSessionState>,
        last_assistant_message: Option<String>,
        timed_out: bool,
        snapshot: FinishSnapshot,
    },
    AwaitingReview {
        site: PathBuf,
        stops: u32,
        before_head: CommitId,
        snapshot: FinishSnapshot,
    },
    AwaitingCredential {
        step: FinishStep,
        site: PathBuf,
        before_head: CommitId,
    },
    NeedsDecision {
        site: PathBuf,
        drift: DriftKind,
    },
    Done(ItemOutcome),
    Skipped(SkipReason),
    Failed {
        message: String,
        output: Option<String>,
    },
}

impl ItemPhase {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done(_) | Self::Skipped(_) | Self::Failed { .. })
    }

    pub fn conflict_site(&self) -> Option<&Path> {
        match self {
            Self::PausedConflict { site, .. }
            | Self::AgentLaunching { site, .. }
            | Self::AgentDispatched { site, .. }
            | Self::AwaitingReview { site, .. }
            | Self::AwaitingCredential { site, .. }
            | Self::NeedsDecision { site, .. } => Some(site.as_path()),
            _ => None,
        }
    }
}

/// One source worktree entry inside a [`WorktreeFinishRun`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinishSource {
    pub path: PathBuf,
    pub branch: String,
    pub locked_head: Option<CommitId>,
    pub initial_snapshot: Option<FinishSnapshot>,
    /// D4: sessions already active in this source worktree when preview ran.
    pub initial_active_sessions: Vec<AgentSession>,
    pub squash: bool,
    pub remove_after: bool,
    pub squash_message: Option<String>,
    pub preview: Loadable<IntegrationPreview>,
    pub phase: ItemPhase,
    /// Expected source HEAD at batch-end cleanup verification.
    pub result_head: Option<CommitId>,
    /// Pre-step `merge-tree` output used to verify clean squash commits (§3.4).
    pub expected_merged_tree: Option<TreeId>,
    pub had_conflict: bool,
    pub conflict_stops: u32,
    pub user_reviewed_resolution: bool,
    pub dispatched_agent: bool,
    /// F5: true when a prior agent launch returned an uncertain/missing receipt or switched away
    /// from an already-launched agent; blocks fresh duplicate launches on this item.
    pub agent_receipt_unknown: bool,
    /// F5: optional `--retry-launch <operationId>` extracted from a lost-reply launch error.
    pub pending_retry_operation_id: Option<String>,
    /// F4: exact index + worktree baseline captured when the conflict paused, used to ensure
    /// Skip/Abort preserves any user files or edits added after the conflict.
    pub conflict_baseline_entries: Option<Vec<WorktreeEntryDigest>>,
    /// Rebase squash range `(oldest, expected_head)` preserved for credential/signing retries.
    pub pending_squash_range: Option<(CommitId, CommitId)>,
    pub cleanup_disqualified: Option<CleanupBlocker>,
    pub cleanup: CleanupOutcome,
    pub cleanup_notes: Vec<String>,
    pub last_git_output: Option<String>,
}

/// Overall phase of a [`WorktreeFinishRun`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunPhase {
    Previewing,
    AwaitingConfirm { fingerprint: u64 },
    Running { current: usize },
    Paused { current: usize },
    CleanupCheck,
    CleaningUp,
    Finished,
    Aborted,
}

/// Orca agent discovery state attached to a run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentDispatchState {
    pub availability: Option<AgentDispatchAvailability>,
    pub installed: Loadable<Vec<InstalledAgent>>,
    pub last_error: Option<String>,
}

impl Default for AgentDispatchState {
    fn default() -> Self {
        Self {
            availability: None,
            installed: Loadable::NotLoaded,
            last_error: None,
        }
    }
}

/// Full in-memory run state for one repository `common_dir` (D2: memory-only, never auto-resumed).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeFinishRun {
    pub id: WorktreeFinishRunId,
    pub origin_repo_id: RepoId,
    pub common_dir: PathBuf,
    pub mode: FinishMode,
    pub target: FinishTarget,
    pub sources: Vec<FinishSource>,
    pub phase: RunPhase,
    pub preview_blockers: Vec<String>,
    pub preview_warnings: Vec<String>,
    pub agent_dispatch: AgentDispatchState,
    pub temp_worktree_kept_note: Option<String>,
    pub cleanup_agent_check_unverified: bool,
    /// Paths that currently have a running embedded GitComet terminal (reported by UI).
    pub running_terminal_paths: Vec<PathBuf>,
    /// Item index to resume from upon `Confirm` (non-zero after mid-batch `RefreshPreview`, F8).
    pub resume_from: usize,
    /// Monotonic request counter correlating in-flight `AgentLaunching` requests (F5).
    pub next_agent_request_id: u64,
    pub last_error: Option<String>,
}

impl WorktreeFinishRun {
    /// Deterministic fingerprint over target, mode, ordered sources, locked heads, and options.
    pub fn compute_fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.mode.hash(&mut hasher);
        self.target.branch.hash(&mut hasher);
        self.target.locked_head.hash(&mut hasher);
        self.resume_from.hash(&mut hasher);
        for source in &self.sources {
            source.path.hash(&mut hasher);
            source.branch.hash(&mut hasher);
            source.locked_head.hash(&mut hasher);
            source.squash.hash(&mut hasher);
            source.remove_after.hash(&mut hasher);
            source.squash_message.hash(&mut hasher);
            if let Some(snap) = &source.initial_snapshot {
                snap.ignored_count.hash(&mut hasher);
                snap.ignored_fingerprint.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.phase, RunPhase::Finished | RunPhase::Aborted)
    }

    /// True only while this run is still an unconfirmed initial draft (before any item has run
    /// and before any temporary target worktree has been created). Mid-batch `RefreshPreview`
    /// transitions back to `Previewing` / `AwaitingConfirm` with `resume_from > 0` or existing
    /// completed items and is NOT an initial draft (F8).
    pub fn is_initial_draft(&self) -> bool {
        matches!(
            self.phase,
            RunPhase::Previewing | RunPhase::AwaitingConfirm { .. }
        ) && self.resume_from == 0
            && !matches!(
                self.target.host,
                TargetHost::Temporary { created: true, .. }
            )
            && self
                .sources
                .iter()
                .all(|s| matches!(s.phase, ItemPhase::Queued))
    }

    pub fn can_confirm(&self) -> bool {
        let remaining = self.sources.get(self.resume_from..).unwrap_or(&[]);
        matches!(self.phase, RunPhase::AwaitingConfirm { .. })
            && self.preview_blockers.is_empty()
            && !remaining.is_empty()
            && self.target.locked_head.is_some()
            && remaining
                .iter()
                .all(|s| s.locked_head.is_some() && matches!(s.preview, Loadable::Ready(_)))
    }

    /// Worktree paths currently owned by this active run (used to disable conflicting Git actions).
    pub fn affected_worktree_paths(&self) -> Vec<PathBuf> {
        if !self.is_active() || self.is_initial_draft() {
            return Vec::new();
        }
        let mut paths = Vec::new();
        if let Some(target_path) = self.target.host.path() {
            paths.push(target_path.to_path_buf());
        }
        match self.phase {
            RunPhase::Previewing
            | RunPhase::AwaitingConfirm { .. }
            | RunPhase::Running { .. }
            | RunPhase::Paused { .. } => {
                for source in &self.sources {
                    if !source.phase.is_terminal() || source.remove_after {
                        paths.push(source.path.clone());
                    }
                }
            }
            RunPhase::CleanupCheck | RunPhase::CleaningUp => {
                for source in &self.sources {
                    if source.remove_after {
                        paths.push(source.path.clone());
                    }
                }
            }
            _ => {}
        }
        paths
    }
}
