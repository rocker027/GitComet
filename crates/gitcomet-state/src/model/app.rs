//! App-wide state: settings, prompts, and notifications.

use super::{CloneOpState, CommitRequest, Loadable, RepoId, RepoState};
use crate::msg::RepoCommandKind;
use gitcomet_core::domain::SignatureFormats;
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::{HistoryRefFilter, SafePushAfterCommitContext, SubmoduleTrustTarget};
use gitcomet_core::signing_tools::SigningToolsState;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SidebarMode {
    #[default]
    Branches,
    Files,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GitLogTagFetchMode {
    #[default]
    OnRepositoryActivation,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitLogSettings {
    pub show_history_tags: bool,
    pub tag_fetch_mode: GitLogTagFetchMode,
    /// Escape hatch: a misconfigured `gpg.program` or a wedged `gpg-agent`
    /// would otherwise slow every history page with no way to turn it off.
    pub verify_commit_signatures: bool,
}

impl Default for GitLogSettings {
    fn default() -> Self {
        Self {
            show_history_tags: true,
            tag_fetch_mode: GitLogTagFetchMode::OnRepositoryActivation,
            verify_commit_signatures: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DefaultTagType {
    #[default]
    Lightweight,
    Annotated,
}

impl GitLogSettings {
    pub fn auto_fetch_tags_on_repo_activation(self) -> bool {
        matches!(
            self.tag_fetch_mode,
            GitLogTagFetchMode::OnRepositoryActivation
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RemoteSettings {
    pub prune_deleted_remote_branches_on_fetch: bool,
    pub auto_fetch: super::AutoFetchSettings,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            prune_deleted_remote_branches_on_fetch: true,
            auto_fetch: super::AutoFetchSettings::default(),
        }
    }
}

/// How GitComet treats Git LFS and git-annex repositories.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LargeFileSettings {
    /// Hide `git-annex` and `synced/*` branches from branch lists.
    pub hide_annex_refs: bool,
    /// On an adjusted branch, Pull and Push run `git annex pull` / `push`:
    /// a plain merge into an adjusted branch is git-annex's documented footgun.
    pub annex_pull_push: bool,
    /// Annex pull, push and sync also move annexed content.
    pub annex_sync_content: bool,
}

impl Default for LargeFileSettings {
    fn default() -> Self {
        Self {
            hide_annex_refs: true,
            annex_pull_push: true,
            annex_sync_content: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenanceSettings {
    /// Check daily whether a repository needs maintenance, and offer to run it.
    pub recommend: bool,
}

impl Default for MaintenanceSettings {
    fn default() -> Self {
        Self { recommend: true }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileBrowserSettings {
    /// Active file browsing follows the selected history row.
    pub follow_selected_commit: bool,
}

impl Default for FileBrowserSettings {
    fn default() -> Self {
        Self {
            follow_selected_commit: true,
        }
    }
}

// ── App state ───────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct AppState {
    pub filesystem: FilesystemState,
    /// Recent failed opens remain observable after their provisional tab is removed.
    pub repository_open_failures:
        std::collections::BTreeMap<PathBuf, (gitcomet_core::filesystem::OperationId, String)>,
    pub repos: Vec<RepoState>,
    pub active_repo: Option<RepoId>,
    /// Unacknowledged open failures, shared by snapshots until they change.
    /// Window routing releases its reservations before acknowledging these.
    pub repo_open_failures: Arc<FxHashMap<PathBuf, u64>>,
    /// Store-wide sequence; acknowledgements must not reset retry baselines.
    pub repo_open_failure_revision: u64,
    pub clone: Option<CloneOpState>,
    /// Notifications the window has not acknowledged yet
    /// (`Msg::AcknowledgeNotifications`).
    pub notifications: Vec<AppNotification>,
    /// Number pushed, including notifications already acknowledged and dropped.
    pub notifications_seq: u64,
    pub auth_prompt: Option<AuthPromptState>,
    /// Every new authentication request has a distinct identity, including
    /// repeated requests for the same key and operation.
    pub auth_prompt_generation: u64,
    /// Cancels local key validation; submitted secrets live only in its effect.
    pub auth_validation_cancellation: Option<gitcomet_core::services::CancellationToken>,
    pub branch_exists_prompt: Option<BranchExistsPromptState>,
    pub force_delete_branch_prompt: Option<ForceDeleteBranchPrompt>,
    /// Every force-delete prompt has a distinct generation, so a refusal that
    /// repeats the last one is still a new prompt.
    pub force_delete_branch_prompt_generation: u64,
    pub force_remove_worktree_prompt: Option<ForceRemoveWorktreePrompt>,
    /// Every force prompt has a distinct generation, so a refusal that
    /// repeats the last one is still a new prompt.
    pub force_remove_worktree_prompt_generation: u64,
    pub submodule_trust_prompt: Option<SubmoduleTrustPromptState>,
    /// A submodule trust check is running in the background. Set the moment the
    /// add/update/load is triggered and cleared when the check resolves, so the
    /// UI can show a pending/spinner state instead of a dead gap before the
    /// trust dialog (or a silent proceed) appears.
    pub submodule_trust_check_pending: Option<SubmoduleTrustCheckState>,
    pub git_runtime: GitRuntimeState,
    /// The signature verifiers Git can run. Formats without one are not verified.
    pub signing_tools: SigningToolsState,
    /// Whether Git can run `git lfs` / `git annex`; gates their commands.
    pub large_file_tools: gitcomet_core::large_file_tools::LargeFileToolsState,
    pub large_file_settings: LargeFileSettings,
    pub remote_url_policy: RemoteUrlPolicy,
    pub git_log_settings: GitLogSettings,
    /// Ref namespaces every all-branches walk leaves out, resolved from
    /// [`HiddenRefSettings`] against the product's presets.
    pub history_hidden_refs: HistoryRefFilter,
    pub remote_settings: RemoteSettings,
    pub maintenance_settings: MaintenanceSettings,
    pub file_browser_settings: FileBrowserSettings,
    pub sidebar_mode: SidebarMode,
    pub default_tag_type: DefaultTagType,
    /// Outstanding [`WatchLease`](crate::store::WatchLease)s per open
    /// repository. A leased repository keeps its file watcher running while
    /// it is not the active one (a hosted view of a linked worktree, say).
    pub watch_leases: Arc<FxHashMap<RepoId, u32>>,
    /// Leases for linked worktrees, scoped to the owning repository lifetime.
    pub worktree_watch_leases: Arc<FxHashMap<(RepoId, u64, std::path::PathBuf), u32>>,
    /// Active or recently finished Worktree Finish runs (at most one active run per `common_dir`).
    pub worktree_finish_runs: Vec<super::WorktreeFinishRun>,
    /// Latest worktree paths reported by the UI as having an open embedded terminal.
    pub running_terminal_paths: Vec<std::path::PathBuf>,
}

impl AppState {
    /// Deterministic fixture: tests opt into an available runtime without spawning Git.
    #[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
    pub fn test_default() -> Self {
        Self {
            git_runtime: GitRuntimeState {
                preference: gitcomet_core::process::GitExecutablePreference::SystemPath,
                availability: gitcomet_core::process::GitExecutableAvailability::Available {
                    version_output: "git version 2.55.0 (test)".into(),
                },
            },
            ..Self::default()
        }
    }

    /// Queues a notification for the window.
    pub fn push_notification(&mut self, notification: AppNotification) {
        self.notifications_seq = self.notifications_seq.wrapping_add(1);
        self.notifications.push(notification);
    }

    /// Notifications queued after notification number `seq`.
    pub fn notifications_after(&self, seq: u64) -> &[AppNotification] {
        super::feedback_after(&self.notifications, self.notifications_seq, seq)
    }

    /// Releases the notifications the window has shown, up to number
    /// `through_seq`.
    pub fn acknowledge_notifications(&mut self, through_seq: u64) {
        super::acknowledge_feedback(&mut self.notifications, self.notifications_seq, through_seq);
    }

    /// Whether `repo_id` belongs to the same Git repository (`common_dir`) as `run`.
    fn repo_matches_worktree_finish_run(
        &self,
        repo_id: RepoId,
        run: &super::WorktreeFinishRun,
    ) -> bool {
        if run.origin_repo_id == repo_id {
            return true;
        }
        let Some(repo) = self.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        if let Some(repo_common) = repo
            .common_dir
            .as_ref()
            .filter(|path| !path.as_os_str().is_empty())
            && !run.common_dir.as_os_str().is_empty()
        {
            return gitcomet_core::path_utils::canonicalize_or_original(repo_common.to_path_buf())
                == gitcomet_core::path_utils::canonicalize_or_original(run.common_dir.clone());
        }
        let repo_workdir =
            gitcomet_core::path_utils::canonicalize_or_original(repo.spec.workdir.clone());
        run.affected_worktree_paths()
            .into_iter()
            .any(|p| gitcomet_core::path_utils::canonicalize_or_original(p) == repo_workdir)
    }

    /// Whether an active Worktree Finish run currently owns `workdir`.
    pub fn worktree_finish_owns_workdir(&self, workdir: &std::path::Path) -> bool {
        let target = gitcomet_core::path_utils::canonicalize_or_original(workdir.to_path_buf());
        self.worktree_finish_runs.iter().any(|run| {
            run.affected_worktree_paths()
                .into_iter()
                .any(|path| gitcomet_core::path_utils::canonicalize_or_original(path) == target)
        })
    }

    /// Whether `repo_id` is currently blocked from history-rewriting or conflicting
    /// checkout/merge/rebase/reset actions by either a Git operation or an active Worktree Finish run.
    pub fn repo_history_rewrite_busy(&self, repo_id: RepoId) -> bool {
        self.repos.iter().any(|repo| {
            repo.id == repo_id
                && (repo.history_rewrite_busy()
                    || self.worktree_finish_owns_workdir(&repo.spec.workdir)
                    || self.worktree_finish_protects_repo_head(repo_id))
        })
    }

    /// Whether an active (non-draft) Worktree Finish run protects local branch `branch`
    /// for `repo_id` (either as target branch or as any source branch) across the shared
    /// canonical `common_dir`.
    pub fn worktree_finish_protects_branch(&self, repo_id: RepoId, branch: &str) -> bool {
        if branch == "HEAD" || branch == "@" {
            return self.worktree_finish_protects_repo_head(repo_id);
        }
        let normalized = branch
            .strip_prefix("refs/heads/")
            .map(|rest| rest.trim_start_matches('/'))
            .unwrap_or(branch);
        if normalized.is_empty() {
            return false;
        }
        self.worktree_finish_runs.iter().any(|run| {
            if !run.is_active() || run.is_initial_draft() {
                return false;
            }
            self.repo_matches_worktree_finish_run(repo_id, run)
                && (run.target.branch == normalized
                    || run.sources.iter().any(|src| src.branch == normalized))
        })
    }

    /// Local branches that active (non-draft) Worktree Finish runs protect in `repo_id`'s
    /// repository: every name [`Self::worktree_finish_protects_branch`] accepts, without
    /// duplicates. Empty when no run is active there.
    pub fn worktree_finish_protected_branches(&self, repo_id: RepoId) -> Vec<String> {
        let mut branches: Vec<String> = self
            .worktree_finish_runs
            .iter()
            .filter(|run| {
                run.is_active()
                    && !run.is_initial_draft()
                    && self.repo_matches_worktree_finish_run(repo_id, run)
            })
            .flat_map(|run| {
                std::iter::once(run.target.branch.clone())
                    .chain(run.sources.iter().map(|src| src.branch.clone()))
            })
            .filter(|branch| !branch.is_empty())
            .collect();
        branches.sort();
        branches.dedup();
        branches
    }

    /// Whether `workdir` is protected from mutation or removal for `repo_id`, including
    /// direct run-owned sites, open tabs checking out a protected branch (such as `RebaseOnto`
    /// target tabs), and linked worktrees whose loaded branch is protected.
    pub fn worktree_finish_protects_worktree(
        &self,
        repo_id: RepoId,
        workdir: &std::path::Path,
    ) -> bool {
        if self.worktree_finish_owns_workdir(workdir) {
            return true;
        }
        let target = gitcomet_core::path_utils::canonicalize_or_original(workdir.to_path_buf());
        if self.repos.iter().any(|repo| {
            gitcomet_core::path_utils::canonicalize_or_original(repo.spec.workdir.clone()) == target
                && self.worktree_finish_protects_repo_head(repo.id)
        }) {
            return true;
        }
        let matching_runs: Vec<&super::WorktreeFinishRun> = self
            .worktree_finish_runs
            .iter()
            .filter(|run| {
                run.is_active()
                    && !run.is_initial_draft()
                    && self.repo_matches_worktree_finish_run(repo_id, run)
            })
            .collect();
        if matching_runs.is_empty() {
            return false;
        }
        self.repos.iter().any(|repo| {
            if !matching_runs
                .iter()
                .any(|run| self.repo_matches_worktree_finish_run(repo.id, run))
            {
                return false;
            }
            let Loadable::Ready(worktrees) = &repo.worktrees else {
                return false;
            };
            worktrees.iter().any(|wt| {
                !wt.bare
                    && !wt.detached
                    && gitcomet_core::path_utils::canonicalize_or_original(wt.path.clone())
                        == target
                    && wt
                        .branch
                        .as_deref()
                        .is_some_and(|branch| self.worktree_finish_protects_branch(repo_id, branch))
            })
        })
    }

    /// Whether `repo_id` currently has a checked-out branch protected by an active Worktree
    /// Finish run (for example, the target branch tab during `RebaseOnto`).
    pub fn worktree_finish_protects_repo_head(&self, repo_id: RepoId) -> bool {
        let Some(repo) = self.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        match &repo.head_branch {
            Loadable::Ready(branch) if !branch.is_empty() && branch != "HEAD" => {
                self.worktree_finish_protects_branch(repo_id, branch)
            }
            _ => false,
        }
    }

    /// Whether `repo_id`'s working tree or checked-out HEAD is protected from mutating actions
    /// by an active Worktree Finish run.
    pub fn worktree_finish_blocks_workdir_mutation(&self, repo_id: RepoId) -> bool {
        let Some(repo) = self.repos.iter().find(|r| r.id == repo_id) else {
            return false;
        };
        self.worktree_finish_owns_workdir(&repo.spec.workdir)
            || self.worktree_finish_protects_repo_head(repo_id)
    }

    /// Whether `repo_id` belongs to a repository (`common_dir`) with an active (non-draft)
    /// Worktree Finish run.
    pub fn has_active_worktree_finish_run(&self, repo_id: RepoId) -> bool {
        self.worktree_finish_runs.iter().any(|run| {
            run.is_active()
                && !run.is_initial_draft()
                && self.repo_matches_worktree_finish_run(repo_id, run)
        })
    }

    /// Most recent non-draft Worktree Finish run belonging to `repo_id`'s repository (`common_dir`).
    pub fn worktree_finish_run_for_repo(
        &self,
        repo_id: RepoId,
    ) -> Option<&super::WorktreeFinishRun> {
        self.worktree_finish_runs.iter().rev().find(|run| {
            !run.is_initial_draft() && self.repo_matches_worktree_finish_run(repo_id, run)
        })
    }

    /// The signature formats to verify: none when the preference is off,
    /// otherwise those whose verifier was not found missing.
    pub fn signature_verification_formats(&self) -> SignatureFormats {
        if self.git_log_settings.verify_commit_signatures {
            self.signing_tools.usable_formats()
        } else {
            SignatureFormats::NONE
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FilesystemState {
    pub pending: std::collections::BTreeSet<gitcomet_core::filesystem::OperationId>,
    pub progress: Option<gitcomet_core::filesystem::Progress>,
    pub completed: std::collections::VecDeque<gitcomet_core::filesystem::OperationResult>,
    pub undo_available: bool,
    pub redo_available: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BranchExistsPromptOperation {
    CreateBranch,
    CheckoutRemoteBranch { remote: String, branch: String },
    RenameBranch { old_name: String },
}

/// `git branch -d` refused a branch that is not fully merged.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ForceDeleteBranchPrompt {
    pub generation: u64,
    pub repo_id: RepoId,
    pub name: String,
}

/// A plain worktree removal Git refused for a reason `--force` overrides.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ForceRemoveWorktreePrompt {
    pub generation: u64,
    pub repo_id: RepoId,
    pub path: PathBuf,
    /// The local branch to delete once the worktree is gone.
    pub delete_branch: Option<String>,
    pub blocker: gitcomet_core::error::WorktreeRemovalBlocker,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchExistsPromptState {
    pub repo_id: RepoId,
    pub name: String,
    pub target: String,
    pub operation: BranchExistsPromptOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthPromptKind {
    UsernamePassword,
    Passphrase,
    HostVerification,
}

impl AuthPromptKind {
    pub fn requires_username(self) -> bool {
        matches!(self, Self::UsernamePassword)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthRetryOperation {
    RepoCommand {
        repo_id: RepoId,
        command: RepoCommandKind,
    },
    SafePushAfterCommit {
        repo_id: RepoId,
        context: SafePushAfterCommitContext,
    },
    Commit {
        repo_id: RepoId,
        request: CommitRequest,
    },
    Clone {
        url: String,
        dest: PathBuf,
    },
    WorktreeFinish {
        run_id: super::WorktreeFinishRunId,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthPromptState {
    pub generation: u64,
    pub challenge: Option<gitcomet_core::auth::AuthChallenge>,
    pub status: AuthPromptStatus,
    pub kind: AuthPromptKind,
    // What the failed command printed, kept when it explains the prompt
    // better than the challenge does: a remote's reason for a rejection.
    pub output: Option<String>,
    pub operation: AuthRetryOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthPromptStatus {
    AwaitingInput,
    Validating,
    Rejected { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmoduleTrustPromptOperation {
    Add {
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
    },
    Update,
    Load {
        path: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmoduleTrustPromptState {
    pub repo_id: RepoId,
    pub operation: SubmoduleTrustPromptOperation,
    pub sources: Vec<SubmoduleTrustTarget>,
}

/// Which pending action a background trust check belongs to. Mirrors the
/// operation so the spinner's title matches the trust dialog that may follow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmoduleTrustCheckOperation {
    Add,
    Update,
    Load,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubmoduleTrustCheckState {
    pub repo_id: RepoId,
    pub operation: SubmoduleTrustCheckOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppNotification {
    pub time: SystemTime,
    pub kind: AppNotificationKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppNotificationKind {
    Info,
    Success,
    Warning,
    Error,
}
