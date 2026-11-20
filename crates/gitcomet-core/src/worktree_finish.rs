//! Contracts for integrating one or more feature worktrees into a target branch.

use crate::domain::CommitId;
use crate::services::{CancellationToken, CommandOutput, Result, SequencerState};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Git tree object identifier in hex form.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TreeId(pub Arc<str>);

impl TreeId {
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

impl AsRef<str> for TreeId {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl From<String> for TreeId {
    fn from(value: String) -> Self {
        Self(Arc::from(value))
    }
}

impl From<&str> for TreeId {
    fn from(value: &str) -> Self {
        Self(Arc::from(value))
    }
}

impl fmt::Display for TreeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_ref())
    }
}

/// Active Git integration state observed inside a worktree.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FinishOperation {
    #[default]
    None,
    Merge,
    /// `git merge --squash` does not create `MERGE_HEAD`; it leaves unmerged
    /// index entries (and usually `SQUASH_MSG`).
    SquashMergeConflict,
    Rebase,
    Other(SequencerState),
}

/// Read-only preview of integrating `source` into/onto `target`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IntegrationPreview {
    /// Commits reachable from `source` that are not reachable from `target`.
    pub ahead: usize,
    /// Commits reachable from `target` that are not reachable from `source`.
    pub behind: usize,
    pub source_is_ancestor_of_target: bool,
    pub target_is_ancestor_of_source: bool,
    /// Tree computed by `git merge-tree --write-tree <target> <source>` when clean.
    pub merged_tree: Option<TreeId>,
    /// Tree of `<target>`.
    pub target_tree: Option<TreeId>,
    /// Conflicting paths reported by `git merge-tree`.
    pub conflict_paths: Vec<PathBuf>,
    /// Whether `target..source` contains no merge commits.
    pub linear: bool,
    /// Commit subjects in `target..source` (oldest first, capped) for default squash messages.
    pub commit_subjects: Vec<String>,
    /// True when `source` has commits ahead of `target` yet merging it produces the exact
    /// same tree as `target` without conflicts (a signal that it may have been squash-landed).
    pub likely_previously_squashed: bool,
    /// Paths changed along `target...source` (used to detect untracked-file overwrite risks).
    pub touched_paths: Vec<PathBuf>,
}

/// Cryptographic verification state for Git-ignored paths inside a worktree.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum IgnoredFilesFingerprint {
    /// Deterministic digest over all ignored paths, file types, permissions/targets, and contents.
    Verified { entry_count: usize, digest: String },
    /// Ignored files could not be reliably verified (budget exceeded, unreadable entry, or unavailable).
    Unverifiable(String),
}

impl IgnoredFilesFingerprint {
    /// Canonical verified fingerprint representing zero ignored files.
    pub fn empty_verified() -> Self {
        Self::Verified {
            entry_count: 0,
            digest: "empty".to_string(),
        }
    }
}

/// Authoritative index and working-tree digest for one tracked or unmerged entry.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct WorktreeEntryDigest {
    pub path: PathBuf,
    pub orig_path: Option<PathBuf>,
    /// Porcelain v2 record kind and XY code (for example `"1 M."`, `"2 R."`, `"u UU"`).
    pub status_code: String,
    /// Object IDs recorded in HEAD/index stages by porcelain v2.
    pub index_identity: String,
    /// Content digest of the working-tree entry (`None` when deleted/absent on disk).
    pub worktree_digest: Option<String>,
}

/// Authoritative Git state snapshot for a single worktree.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FinishSnapshot {
    pub worktree_exists: bool,
    pub is_primary_worktree: bool,
    pub is_bare: bool,
    pub head: Option<CommitId>,
    pub head_parents: Vec<CommitId>,
    pub head_tree: Option<TreeId>,
    pub branch: Option<String>,
    pub detached: bool,
    pub operation: FinishOperation,
    pub has_merge_head: bool,
    pub has_squash_msg: bool,
    pub unmerged: Vec<PathBuf>,
    pub conflict_marker_paths: Vec<PathBuf>,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
    pub untracked_paths: Vec<PathBuf>,
    pub ignored_count: usize,
    pub ignored_sample: Vec<PathBuf>,
    pub ignored_fingerprint: Option<IgnoredFilesFingerprint>,
    /// Exact tracked/unmerged index + worktree digests; `None` means unverified (fail-closed for abort).
    pub dirty_entries: Option<Vec<WorktreeEntryDigest>>,
    pub has_initialized_submodules: bool,
    pub locked: bool,
}

impl FinishSnapshot {
    /// True when there are no staged, unstaged, or unmerged tracked changes.
    pub fn is_clean_tracked(&self) -> bool {
        self.staged_count == 0 && self.unstaged_count == 0 && self.unmerged.is_empty()
    }

    /// True when there are no tracked or untracked changes.
    pub fn is_completely_clean(&self) -> bool {
        self.is_clean_tracked() && self.untracked_count == 0
    }

    /// True when `merge --squash` staged changes (with or without `SQUASH_MSG`)
    /// and no unmerged entries remain.
    pub fn has_staged_squash(&self) -> bool {
        self.operation == FinishOperation::None
            && !self.has_merge_head
            && self.has_squash_msg
            && self.unmerged.is_empty()
            && self.staged_count > 0
    }
}

/// Outcome of running a raw Git integration step where non-zero exit codes
/// (such as conflict stops or `rerere.autoUpdate`) must be inspected alongside
/// the post-step [`FinishSnapshot`] rather than blindly discarded.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FinishStepCommandOutput {
    pub output: CommandOutput,
    pub succeeded: bool,
}

impl FinishStepCommandOutput {
    pub fn from_output(output: CommandOutput) -> Self {
        let succeeded = output.exit_code == Some(0);
        Self { output, succeeded }
    }
}

/// Availability of an external agent dispatcher (such as the Orca CLI).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentDispatchAvailability {
    Available { executable: PathBuf },
    NotFound,
    Failed(String),
}

/// Dynamically discovered agent entry returned by `orca agent installed`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledAgent {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Lifecycle state reported for an external agent session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentSessionState {
    Working,
    Blocked,
    Waiting,
    Done,
    Unknown(String),
}

impl AgentSessionState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Working | Self::Blocked | Self::Waiting)
    }

    pub fn triggers_recheck(&self) -> bool {
        matches!(self, Self::Done | Self::Blocked | Self::Waiting)
    }
}

/// Launch receipt returned by `orca agent launch`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentLaunch {
    pub session_id: Option<String>,
    pub operation_id: Option<String>,
    pub outcome_kind: Option<String>,
    pub detail: Option<String>,
}

/// Observed session for a workspace returned by `orca agent list`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSession {
    pub session_id: String,
    pub agent_id: String,
    pub state: AgentSessionState,
    pub worktree_id: Option<String>,
    pub updated_at: Option<String>,
    /// Untrusted plain text for display only; never parsed for control flow.
    pub last_assistant_message: Option<String>,
}

/// Injectable external agent dispatcher for conflict resolution and occupancy checks.
pub trait AgentDispatcher: Send + Sync {
    fn availability(&self) -> AgentDispatchAvailability;

    fn installed(
        &self,
        workspace: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Vec<InstalledAgent>>;

    fn launch(
        &self,
        workspace: &Path,
        agent_id: &str,
        prompt: &str,
        retry_operation_id: Option<&str>,
    ) -> Result<AgentLaunch>;

    fn sessions(
        &self,
        workspace: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Vec<AgentSession>>;
}

/// Builds the fixed conflict-resolution prompt sent to an Orca agent.
pub fn conflict_resolution_prompt(
    site: &Path,
    operation_label: &str,
    conflict_paths: &[PathBuf],
) -> String {
    let mut files = String::new();
    for path in conflict_paths {
        files.push_str("- ");
        files.push_str(&path.to_string_lossy());
        files.push('\n');
    }
    if files.is_empty() {
        files.push_str("(run `git status` to inspect unmerged files)\n");
    }
    format!(
        "You are working in {site}.\n\
         There is an in-progress {op} with conflicts in the following files:\n\
         {files}\
         Only do this task: resolve the conflict markers in those files and run `git add` on the resolved files.\n\
         Do NOT commit, do NOT run `git merge --continue/--abort` or `git rebase --continue/--abort`, do NOT switch branches, and do NOT modify files outside this list.\n\
         When finished, report which files you resolved and your reasoning.",
        site = site.display(),
        op = operation_label,
    )
}

/// Builds a default commit message for squash-merging `source_branch` into `target_branch`.
pub fn default_squash_merge_message(
    source_branch: &str,
    target_branch: &str,
    subjects: &[String],
) -> String {
    let mut message = format!("Squash merge {source_branch} into {target_branch}");
    if !subjects.is_empty() {
        message.push_str("\n\n");
        for subject in subjects {
            let trimmed = subject.trim();
            if !trimmed.is_empty() {
                message.push_str("- ");
                message.push_str(trimmed);
                message.push('\n');
            }
        }
        while message.ends_with('\n') {
            message.pop();
        }
    }
    message
}

/// Default merge commit message matching Git's branch merge phrasing when merging by SHA.
pub fn default_merge_branch_message(source_branch: &str, target_branch: &str) -> String {
    format!("Merge branch '{source_branch}' into {target_branch}")
}

/// Truncates untrusted assistant text to a safe display length.
pub fn sanitize_assistant_message(raw: &str, max_chars: usize) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let prefix: String = trimmed.chars().take(max_chars).collect();
    format!("{prefix}…")
}
