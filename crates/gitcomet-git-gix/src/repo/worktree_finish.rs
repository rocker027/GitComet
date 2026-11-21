use super::GixRepo;
use crate::util::{
    command_output, git_command_failed_error, git_command_timeout, path_buf_from_git_bytes,
    run_command_with_timeout, run_git_capture_bytes_cancellable, run_git_with_output,
    validate_hex_commit_id, validate_ref_like_arg,
};
use gitcomet_core::conflict_session::reader_has_conflict_markers;
use gitcomet_core::domain::{CommitId, RepositoryMode};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{
    CancellationToken, CommandOutput, GitBackend as _, GitHooks, Result, SequencerState,
};
use gitcomet_core::worktree_finish::{
    FinishOperation, FinishSnapshot, FinishStepCommandOutput, IgnoredFilesFingerprint,
    IntegrationPreview, TreeId, WorktreeEntryDigest,
};
use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Read as _};
use std::path::{Path, PathBuf};

pub(super) const NO_COLOR_CONFIG: [&str; 2] = ["-c", "color.ui=false"];
const MAX_PREVIEW_SUBJECTS: usize = 32;
const MAX_UNTRACKED_PATHS: usize = 512;
const MAX_IGNORED_SAMPLE: usize = 32;
const MAX_IGNORED_VERIFY_FILES: usize = 4096;
const MAX_IGNORED_VERIFY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IGNORED_VERIFY_DEPTH: usize = 64;
const MAX_DIRTY_VERIFY_ENTRIES: usize = 2048;
const MAX_DIRTY_VERIFY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CONFLICT_MARKER_SCAN_FILES: usize = 256;
const MAX_CONFLICT_MARKER_SCAN_BYTES: u64 = 8 * 1024 * 1024;

impl GixRepo {
    pub(super) fn integration_preview_impl(
        &self,
        target: &CommitId,
        source: &CommitId,
        cancellation: &CancellationToken,
    ) -> Result<IntegrationPreview> {
        cancellation.check_cancelled()?;
        validate_hex_commit_id(target)?;
        validate_hex_commit_id(source)?;

        let (behind, ahead) = self.rev_list_left_right_count(target, source, cancellation)?;
        cancellation.check_cancelled()?;

        let source_is_ancestor_of_target =
            self.is_commit_ancestor_cancellable(source, target, cancellation)?;
        let target_is_ancestor_of_source =
            self.is_commit_ancestor_cancellable(target, source, cancellation)?;
        cancellation.check_cancelled()?;

        let target_tree = self.commit_tree_id(target, cancellation)?;
        let (merged_tree, conflict_paths) =
            self.merge_tree_preview(target, source, cancellation)?;
        cancellation.check_cancelled()?;

        let linear = self.range_is_linear(target, source, cancellation)?;
        let commit_subjects = self.range_commit_subjects(target, source, cancellation)?;
        let touched_paths = self.range_touched_paths(target, source, cancellation)?;
        cancellation.check_cancelled()?;

        let likely_previously_squashed = ahead > 0
            && !source_is_ancestor_of_target
            && conflict_paths.is_empty()
            && merged_tree.is_some()
            && merged_tree == target_tree;

        Ok(IntegrationPreview {
            ahead,
            behind,
            source_is_ancestor_of_target,
            target_is_ancestor_of_source,
            merged_tree,
            target_tree,
            conflict_paths,
            linear,
            commit_subjects,
            likely_previously_squashed,
            touched_paths,
        })
    }

    pub(super) fn finish_snapshot_impl(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<FinishSnapshot> {
        cancellation.check_cancelled()?;
        let workdir = &self.spec.workdir;
        if !workdir.exists() {
            return Ok(FinishSnapshot {
                worktree_exists: false,
                ..Default::default()
            });
        }

        let is_bare = self.repository_mode_impl()? == RepositoryMode::Bare;
        let (is_primary_worktree, locked) = self.worktree_primary_and_locked(cancellation)?;
        cancellation.check_cancelled()?;

        let (head, head_tree, head_parents) = self.head_commit_metadata(cancellation)?;
        let (branch, detached) = self.head_branch_state(head.is_some(), cancellation)?;
        cancellation.check_cancelled()?;

        let repo = self.repo();
        let git_dir = repo.path().to_path_buf();
        // Ref-dependent state goes through the refs storage adapter. A rebase or cherry-pick
        // outranks a merge there, and `classify_operation` reports those first as well.
        let has_merge_head =
            crate::refs::operation_state(&repo)? == Some(gix::state::InProgress::Merge);
        let has_squash_msg = git_dir.join("SQUASH_MSG").is_file();
        let sequencer = self.sequencer_state_impl()?;

        if is_bare {
            let operation = classify_operation(sequencer, has_merge_head, false);
            return Ok(FinishSnapshot {
                worktree_exists: true,
                is_primary_worktree,
                is_bare: true,
                head,
                head_parents,
                head_tree,
                branch,
                detached,
                operation,
                has_merge_head,
                has_squash_msg,
                locked,
                ..Default::default()
            });
        }

        let status_summary = self.porcelain_v2_status_summary(cancellation)?;
        cancellation.check_cancelled()?;

        let operation = classify_operation(
            sequencer,
            has_merge_head,
            !status_summary.unmerged.is_empty(),
        );

        let conflict_marker_paths = if operation != FinishOperation::None || has_squash_msg {
            scan_conflict_markers(workdir, &status_summary.candidate_marker_paths)
        } else {
            Vec::new()
        };

        let ignored_fingerprint = Some(compute_ignored_fingerprint(
            workdir,
            &status_summary.ignored_all_paths,
            status_summary.ignored_paths_truncated,
            cancellation,
        )?);
        cancellation.check_cancelled()?;

        let dirty_entries = build_dirty_entries(
            workdir,
            &status_summary.raw_dirty_entries,
            status_summary.dirty_entries_truncated,
            cancellation,
        )?;
        cancellation.check_cancelled()?;

        let has_initialized_submodules = self.has_initialized_submodules_impl(cancellation)?;
        cancellation.check_cancelled()?;

        Ok(FinishSnapshot {
            worktree_exists: true,
            is_primary_worktree,
            is_bare: false,
            head,
            head_parents,
            head_tree,
            branch,
            detached,
            operation,
            has_merge_head,
            has_squash_msg,
            unmerged: status_summary.unmerged,
            conflict_marker_paths,
            staged_count: status_summary.staged_count,
            unstaged_count: status_summary.unstaged_count,
            untracked_count: status_summary.untracked_count,
            untracked_paths: status_summary.untracked_paths,
            ignored_count: status_summary.ignored_count,
            ignored_sample: status_summary.ignored_sample,
            ignored_fingerprint,
            dirty_entries,
            has_initialized_submodules,
            locked,
        })
    }

    pub(super) fn merge_commit_at_impl(
        &self,
        reference: &CommitId,
        message: &str,
    ) -> Result<FinishStepCommandOutput> {
        validate_hex_commit_id(reference)?;
        let label = format!(
            "git merge --ff --no-edit -m <message> -- {}",
            reference.as_ref()
        );
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .arg("merge")
            .arg("--ff")
            .arg("--no-edit")
            .arg("-m")
            .arg(message)
            .arg("--")
            .arg(reference.as_ref());
        run_step_preserving_auth_error(cmd, &label)
    }

    pub(super) fn squash_merge_at_impl(
        &self,
        reference: &CommitId,
    ) -> Result<FinishStepCommandOutput> {
        validate_hex_commit_id(reference)?;
        let label = format!("git merge --squash --no-commit -- {}", reference.as_ref());
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .arg("merge")
            .arg("--squash")
            .arg("--no-commit")
            .arg("--")
            .arg(reference.as_ref());
        run_step_preserving_auth_error(cmd, &label)
    }

    pub(super) fn rebase_onto_at_impl(&self, target: &CommitId) -> Result<FinishStepCommandOutput> {
        validate_hex_commit_id(target)?;
        let label = format!(
            "git rebase --no-rebase-merges --no-update-refs --no-autosquash --no-autostash -- {}",
            target.as_ref()
        );
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .env("GIT_EDITOR", "true")
            .arg("rebase")
            .arg("--no-rebase-merges")
            .arg("--no-update-refs")
            .arg("--no-autosquash")
            .arg("--no-autostash")
            .arg("--")
            .arg(target.as_ref());
        run_step_preserving_auth_error(cmd, &label)
    }

    pub(super) fn rebase_continue_step_impl(&self) -> Result<FinishStepCommandOutput> {
        let label = "git rebase --continue";
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .env("GIT_EDITOR", "true")
            .arg("rebase")
            .arg("--continue");
        run_step_preserving_auth_error(cmd, label)
    }

    pub(super) fn abort_squash_merge_impl(&self) -> Result<CommandOutput> {
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .arg("reset")
            .arg("--merge");
        let output = run_git_with_output(cmd, "git reset --merge")?;

        let squash_msg = self.repo().path().join("SQUASH_MSG");
        if squash_msg.is_file() {
            let _ = std::fs::remove_file(&squash_msg);
        }
        Ok(output)
    }

    fn literal_pathspec_git_cmd(&self) -> std::process::Command {
        let mut cmd = self.git_workdir_cmd();
        // Git rejects `--literal-pathspecs` if any conflicting global pathspec env var is inherited.
        cmd.env_remove("GIT_GLOB_PATHSPECS")
            .env_remove("GIT_NOGLOB_PATHSPECS")
            .env_remove("GIT_ICASE_PATHSPECS");
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .arg("--literal-pathspecs");
        cmd
    }

    pub(super) fn abort_squash_merge_paths_impl(
        &self,
        owned_paths: &[PathBuf],
    ) -> Result<CommandOutput> {
        let unique_path_set: BTreeSet<PathBuf> = owned_paths.iter().cloned().collect();
        if unique_path_set.is_empty() {
            return Err(Error::new(ErrorKind::Backend(
                "Refusing scoped squash abort with empty owned path list; preserving index and working tree."
                    .to_string(),
            )));
        }
        for path in &unique_path_set {
            if !is_safe_literal_repo_file_path(path) {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "Refusing scoped squash abort for non-literal or unsafe path '{}'; preserving index and working tree.",
                    path.display()
                ))));
            }
        }
        let unique_paths: Vec<PathBuf> = unique_path_set.iter().cloned().collect();

        let mut combined_stdout = String::new();
        let mut combined_stderr = String::new();

        // F4: every query and write MUST use literal pathspecs so filenames such as `*`, `?`,
        // `[...]`, `:(glob)*`, `:(icase)...`, or `:!...` never expand to unrelated files.
        let mut ls_cmd = self.literal_pathspec_git_cmd();
        ls_cmd
            .arg("ls-tree")
            .arg("-r")
            .arg("-z")
            .arg("HEAD")
            .arg("--");
        for path in &unique_paths {
            ls_cmd.arg(path);
        }
        let ls_bytes = run_git_capture_bytes_cancellable(
            ls_cmd,
            "git --literal-pathspecs ls-tree -r -z HEAD",
            &CancellationToken::new(),
        )?;
        let mut in_head_set = BTreeSet::new();
        for raw in ls_bytes.split(|b| *b == 0) {
            if raw.is_empty() {
                continue;
            }
            let Some(tab_pos) = raw.iter().position(|b| *b == b'\t') else {
                return Err(Error::new(ErrorKind::Backend(
                    "Scoped squash abort refused: unexpected HEAD ls-tree record format; preserving index and working tree."
                        .to_string(),
                )));
            };
            let meta = std::str::from_utf8(&raw[..tab_pos]).map_err(|_| {
                Error::new(ErrorKind::Backend(
                    "Scoped squash abort refused: invalid UTF-8 in HEAD ls-tree metadata; preserving index and working tree."
                        .to_string(),
                ))
            })?;
            let mut parts = meta.split_ascii_whitespace();
            let _mode = parts.next();
            let obj_type = parts.next();
            let _oid = parts.next();
            if obj_type != Some("blob") || parts.next().is_some() {
                return Err(Error::new(ErrorKind::Backend(
                    "Scoped squash abort refused: HEAD entry is not a regular blob file; preserving index and working tree."
                        .to_string(),
                )));
            }
            let matched = path_buf_from_git_bytes(&raw[tab_pos + 1..], "git ls-tree HEAD path")?;
            if !unique_path_set.contains(&matched) {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "Scoped squash abort refused: HEAD lookup matched unowned path '{}'; preserving index and working tree.",
                    matched.display()
                ))));
            }
            in_head_set.insert(matched);
        }

        // Verify the index query matches ONLY exact owned literal file paths before any write.
        let mut ls_files_cmd = self.literal_pathspec_git_cmd();
        ls_files_cmd.arg("ls-files").arg("-z").arg("--");
        for path in &unique_paths {
            ls_files_cmd.arg(path);
        }
        let ls_files_bytes = run_git_capture_bytes_cancellable(
            ls_files_cmd,
            "git --literal-pathspecs ls-files -z",
            &CancellationToken::new(),
        )?;
        for raw in ls_files_bytes.split(|b| *b == 0) {
            if raw.is_empty() {
                continue;
            }
            let matched = path_buf_from_git_bytes(raw, "git ls-files index path")?;
            if !unique_path_set.contains(&matched) {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "Scoped squash abort refused: index lookup matched unowned path '{}'; preserving index and working tree.",
                    matched.display()
                ))));
            }
        }

        let mut in_head = Vec::new();
        let mut not_in_head = Vec::new();
        for path in unique_paths {
            if in_head_set.contains(&path) {
                in_head.push(path);
            } else {
                not_in_head.push(path);
            }
        }

        // Refuse disk directory collisions before mutating the index, working tree, or SQUASH_MSG.
        for path in &not_in_head {
            if let Ok(meta) = std::fs::symlink_metadata(self.spec.workdir.join(path))
                && meta.file_type().is_dir()
            {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "Scoped squash abort refused: '{}' is a directory on disk; preserving index and working tree.",
                    path.display()
                ))));
            }
        }
        // Restoring an owned HEAD file replaces whatever occupies its path or its parent
        // directories, including ignored and untracked data `git status` never reports. Prove
        // every such entry is a squash-added owned path (removed by `git rm` below) before the
        // first write; otherwise refuse the whole abort so nothing is partially restored.
        let owned_removals: BTreeSet<&Path> = not_in_head.iter().map(PathBuf::as_path).collect();
        for path in &in_head {
            if let Ok(meta) = std::fs::symlink_metadata(self.spec.workdir.join(path))
                && meta.file_type().is_dir()
                && let Some(unowned) =
                    first_unowned_entry_in_dir(&self.spec.workdir, path, &owned_removals)
            {
                return Err(Error::new(ErrorKind::Backend(format!(
                    "Scoped squash abort refused: restoring '{}' as a file would delete '{}', which this squash does not own; preserving index, working tree and conflict state for manual recovery.",
                    path.display(),
                    unowned.display()
                ))));
            }
            for ancestor in path.ancestors().skip(1) {
                if ancestor.as_os_str().is_empty() {
                    break;
                }
                if let Ok(meta) = std::fs::symlink_metadata(self.spec.workdir.join(ancestor))
                    && !meta.file_type().is_dir()
                    && !owned_removals.contains(ancestor)
                {
                    return Err(Error::new(ErrorKind::Backend(format!(
                        "Scoped squash abort refused: restoring '{}' would replace '{}', which this squash does not own; preserving index, working tree and conflict state for manual recovery.",
                        path.display(),
                        ancestor.display()
                    ))));
                }
            }
        }

        if !not_in_head.is_empty() {
            let mut rm_cmd = self.literal_pathspec_git_cmd();
            rm_cmd.arg("rm").arg("-f").arg("--ignore-unmatch").arg("--");
            for path in &not_in_head {
                rm_cmd.arg(path);
            }
            let out =
                run_git_with_output(rm_cmd, "git --literal-pathspecs rm -f -- <squash-added>")?;
            if !out.stdout.is_empty() {
                combined_stdout.push_str(&out.stdout);
            }
            if !out.stderr.is_empty() {
                combined_stderr.push_str(&out.stderr);
            }
        }

        if !in_head.is_empty() {
            let mut checkout_cmd = self.literal_pathspec_git_cmd();
            checkout_cmd.arg("checkout").arg("HEAD").arg("--");
            for path in &in_head {
                checkout_cmd.arg(path);
            }
            let out = run_git_with_output(
                checkout_cmd,
                "git --literal-pathspecs checkout HEAD -- <owned>",
            )?;
            if !out.stdout.is_empty() {
                combined_stdout.push_str(&out.stdout);
            }
            if !out.stderr.is_empty() {
                combined_stderr.push_str(&out.stderr);
            }
        }

        let squash_msg = self.repo().path().join("SQUASH_MSG");
        if squash_msg.is_file() {
            let _ = std::fs::remove_file(&squash_msg);
        }

        Ok(CommandOutput {
            command: "abort squash merge (owned paths)".to_string(),
            stdout: combined_stdout,
            stderr: combined_stderr,
            exit_code: Some(0),
        })
    }

    pub(super) fn conclude_merge_impl(&self, message: Option<&str>) -> Result<CommandOutput> {
        if let Some(msg) = message {
            self.commit_with_identity_impl(msg, None, false, GitHooks::Run)?;
            let squash_msg = self.repo().path().join("SQUASH_MSG");
            if squash_msg.is_file() {
                let _ = std::fs::remove_file(&squash_msg);
            }
            return Ok(CommandOutput::empty_success("git commit"));
        }
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .arg("commit")
            .arg("--no-edit");
        run_git_with_output(cmd, "git commit --no-edit")
    }

    pub(super) fn remove_worktree_checked_impl(
        &self,
        path: &Path,
        expected_head: &CommitId,
    ) -> Result<CommandOutput> {
        validate_hex_commit_id(expected_head)?;
        let target_repo = crate::GixBackend.open(path)?;
        let snapshot = target_repo.finish_snapshot(&CancellationToken::new())?;
        if !snapshot.worktree_exists {
            return Err(Error::new(ErrorKind::Backend(
                "worktree directory no longer exists".to_string(),
            )));
        }
        if snapshot.is_primary_worktree {
            return Err(Error::new(ErrorKind::Backend(
                "cannot remove primary worktree".to_string(),
            )));
        }
        if snapshot.is_bare {
            return Err(Error::new(ErrorKind::Backend(
                "cannot remove bare worktree".to_string(),
            )));
        }
        if snapshot.locked {
            return Err(Error::new(ErrorKind::Backend(
                "worktree is locked".to_string(),
            )));
        }
        if snapshot.has_initialized_submodules {
            return Err(Error::new(ErrorKind::Backend(
                "worktree contains initialized submodules".to_string(),
            )));
        }
        if snapshot.operation != FinishOperation::None {
            return Err(Error::new(ErrorKind::Backend(
                "worktree has an in-progress Git operation".to_string(),
            )));
        }
        if snapshot.detached {
            return Err(Error::new(ErrorKind::Backend(
                "worktree HEAD is detached".to_string(),
            )));
        }
        if snapshot.head.as_ref() != Some(expected_head) {
            return Err(Error::new(ErrorKind::Backend(
                "worktree HEAD changed since integration completed".to_string(),
            )));
        }
        if !snapshot.is_completely_clean() {
            return Err(Error::new(ErrorKind::Backend(
                "worktree has uncommitted or untracked changes".to_string(),
            )));
        }
        // Never passes `--force` and never deletes the branch ref.
        self.remove_worktree_with_output_impl(path)
    }

    pub(super) fn resolve_ref_commit_impl(&self, reference: &str) -> Result<Option<CommitId>> {
        validate_ref_like_arg(reference, "reference")?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-parse")
            .arg("--verify")
            .arg("--quiet")
            .arg(format!("{reference}^{{commit}}"));
        let output =
            run_command_with_timeout(cmd, "git rev-parse --verify", git_command_timeout(), None)?;
        if !output.status.success() {
            return Ok(None);
        }
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if sha.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CommitId(sha.into())))
        }
    }

    pub(super) fn linear_range_after_impl(
        &self,
        base: &CommitId,
        head: &CommitId,
    ) -> Result<Option<(CommitId, usize)>> {
        validate_hex_commit_id(base)?;
        validate_hex_commit_id(head)?;
        let range = format!("{}..{}", base.as_ref(), head.as_ref());
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-list")
            .arg("--first-parent")
            .arg("--reverse")
            .arg(&range);
        let bytes = run_git_capture_bytes_cancellable(
            cmd,
            "git rev-list --first-parent --reverse",
            &CancellationToken::new(),
        )?;
        let text = String::from_utf8_lossy(&bytes);
        let commits: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        let Some(oldest) = commits.first() else {
            return Ok(None);
        };
        Ok(Some((CommitId((*oldest).into()), commits.len())))
    }

    pub(super) fn is_commit_ancestor_impl(
        &self,
        ancestor: &CommitId,
        descendant: &CommitId,
    ) -> Result<bool> {
        self.is_commit_ancestor_cancellable(ancestor, descendant, &CancellationToken::new())
    }

    fn is_commit_ancestor_cancellable(
        &self,
        ancestor: &CommitId,
        descendant: &CommitId,
        cancellation: &CancellationToken,
    ) -> Result<bool> {
        validate_hex_commit_id(ancestor)?;
        validate_hex_commit_id(descendant)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("merge-base")
            .arg("--is-ancestor")
            .arg(ancestor.as_ref())
            .arg(descendant.as_ref());
        let output = run_command_with_timeout(
            cmd,
            "git merge-base --is-ancestor",
            git_command_timeout(),
            Some(cancellation),
        )?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(git_command_failed_error(
                "git merge-base --is-ancestor",
                output,
            )),
        }
    }

    fn rev_list_left_right_count(
        &self,
        target: &CommitId,
        source: &CommitId,
        cancellation: &CancellationToken,
    ) -> Result<(usize, usize)> {
        let range = format!("{}...{}", target.as_ref(), source.as_ref());
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-list")
            .arg("--left-right")
            .arg("--count")
            .arg(&range);
        let bytes = run_git_capture_bytes_cancellable(
            cmd,
            "git rev-list --left-right --count",
            cancellation,
        )?;
        let text = String::from_utf8_lossy(&bytes);
        let mut parts = text.split_whitespace();
        let behind = parts
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let ahead = parts
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        Ok((behind, ahead))
    }

    fn commit_tree_id(
        &self,
        commit: &CommitId,
        cancellation: &CancellationToken,
    ) -> Result<Option<TreeId>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-parse")
            .arg("--verify")
            .arg(format!("{}^{{tree}}", commit.as_ref()));
        let bytes = run_git_capture_bytes_cancellable(cmd, "git rev-parse tree", cancellation)?;
        let sha = String::from_utf8_lossy(&bytes).trim().to_string();
        Ok((!sha.is_empty()).then(|| TreeId::from(sha)))
    }

    fn merge_tree_preview(
        &self,
        target: &CommitId,
        source: &CommitId,
        cancellation: &CancellationToken,
    ) -> Result<(Option<TreeId>, Vec<PathBuf>)> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("merge-tree")
            .arg("--write-tree")
            .arg("-z")
            .arg("--name-only")
            .arg(target.as_ref())
            .arg(source.as_ref());
        let output = run_command_with_timeout(
            cmd,
            "git merge-tree --write-tree -z --name-only",
            git_command_timeout(),
            Some(cancellation),
        )?;
        match output.status.code() {
            Some(0) => {
                let first = output.stdout.split(|b| *b == 0).next().unwrap_or_default();
                let tree = String::from_utf8_lossy(first).trim().to_string();
                Ok(((!tree.is_empty()).then(|| TreeId::from(tree)), Vec::new()))
            }
            Some(1) => {
                let conflict_paths = parse_merge_tree_conflict_paths(&output.stdout)?;
                Ok((None, conflict_paths))
            }
            _ => Err(git_command_failed_error(
                "git merge-tree --write-tree -z --name-only",
                output,
            )),
        }
    }

    fn range_is_linear(
        &self,
        target: &CommitId,
        source: &CommitId,
        cancellation: &CancellationToken,
    ) -> Result<bool> {
        let range = format!("{}..{}", target.as_ref(), source.as_ref());
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("rev-list")
            .arg("--merges")
            .arg("--count")
            .arg(&range);
        let bytes =
            run_git_capture_bytes_cancellable(cmd, "git rev-list --merges --count", cancellation)?;
        let count = String::from_utf8_lossy(&bytes)
            .trim()
            .parse::<usize>()
            .unwrap_or(0);
        Ok(count == 0)
    }

    fn range_commit_subjects(
        &self,
        target: &CommitId,
        source: &CommitId,
        cancellation: &CancellationToken,
    ) -> Result<Vec<String>> {
        let range = format!("{}..{}", target.as_ref(), source.as_ref());
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("log")
            .arg("--reverse")
            .arg(format!("--max-count={MAX_PREVIEW_SUBJECTS}"))
            .arg("--format=%s")
            .arg(&range);
        let bytes = run_git_capture_bytes_cancellable(cmd, "git log --format=%s", cancellation)?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    fn range_touched_paths(
        &self,
        target: &CommitId,
        source: &CommitId,
        cancellation: &CancellationToken,
    ) -> Result<Vec<PathBuf>> {
        let range = format!("{}...{}", target.as_ref(), source.as_ref());
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("diff").arg("--name-only").arg("-z").arg(&range);
        let bytes =
            run_git_capture_bytes_cancellable(cmd, "git diff --name-only -z", cancellation)?;
        let mut paths = Vec::new();
        for raw in bytes.split(|b| *b == 0) {
            if raw.is_empty() {
                continue;
            }
            paths.push(path_buf_from_git_bytes(raw, "git diff --name-only -z")?);
        }
        Ok(paths)
    }

    fn worktree_primary_and_locked(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(bool, bool)> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("worktree").arg("list").arg("--porcelain").arg("-z");
        let bytes = run_git_capture_bytes_cancellable(
            cmd,
            "git worktree list --porcelain -z",
            cancellation,
        )?;
        let self_path = canonicalize_or_original(self.spec.workdir.clone());

        let mut is_first = true;
        let mut current_path: Option<PathBuf> = None;
        let mut current_locked = false;

        let check_record =
            |path: Option<PathBuf>, locked: bool, primary: bool| -> Option<(bool, bool)> {
                let path = path?;
                if canonicalize_or_original(path) == self_path {
                    Some((primary, locked))
                } else {
                    None
                }
            };

        for field in bytes.split(|b| *b == 0) {
            if field.is_empty() {
                if let Some(found) = check_record(current_path.take(), current_locked, is_first) {
                    return Ok(found);
                }
                is_first = false;
                current_locked = false;
                continue;
            }
            if let Some(rest) = field.strip_prefix(b"worktree ") {
                current_path = Some(path_buf_from_git_bytes(rest, "git worktree list")?);
            } else if field == b"locked" || field.starts_with(b"locked ") {
                current_locked = true;
            }
        }
        if let Some(found) = check_record(current_path.take(), current_locked, is_first) {
            return Ok(found);
        }
        Ok((false, false))
    }

    fn head_commit_metadata(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(Option<CommitId>, Option<TreeId>, Vec<CommitId>)> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("log")
            .arg("-1")
            .arg("--format=%H%n%T%n%P")
            .arg("HEAD");
        let output = run_command_with_timeout(
            cmd,
            "git log -1 HEAD metadata",
            git_command_timeout(),
            Some(cancellation),
        )?;
        if !output.status.success() {
            return Ok((None, None, Vec::new()));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        let head = lines
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| CommitId(s.into()));
        let tree = lines
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(TreeId::from);
        let parents = lines
            .next()
            .map(|line| {
                line.split_whitespace()
                    .filter(|s| !s.is_empty())
                    .map(|s| CommitId(s.into()))
                    .collect()
            })
            .unwrap_or_default();
        Ok((head, tree, parents))
    }

    fn head_branch_state(
        &self,
        has_head: bool,
        cancellation: &CancellationToken,
    ) -> Result<(Option<String>, bool)> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("symbolic-ref")
            .arg("--quiet")
            .arg("--short")
            .arg("HEAD");
        let output = run_command_with_timeout(
            cmd,
            "git symbolic-ref --short HEAD",
            git_command_timeout(),
            Some(cancellation),
        )?;
        if output.status.success() {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !name.is_empty() {
                return Ok((Some(name), false));
            }
        }
        Ok((None, has_head))
    }

    fn porcelain_v2_status_summary(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<StatusSummary> {
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("status")
            .arg("--porcelain=v2")
            .arg("-z")
            .arg("--ignored=traditional")
            .arg("--untracked-files=all");
        let bytes = run_git_capture_bytes_cancellable(
            cmd,
            "git status --porcelain=v2 -z --ignored=traditional",
            cancellation,
        )?;
        parse_porcelain_v2_status(&bytes)
    }

    fn has_initialized_submodules_impl(&self, cancellation: &CancellationToken) -> Result<bool> {
        if !self.spec.workdir.join(".gitmodules").is_file() {
            return Ok(false);
        }
        let mut cmd = self.git_workdir_cmd();
        cmd.arg("submodule").arg("status");
        let output = run_command_with_timeout(
            cmd,
            "git submodule status",
            git_command_timeout(),
            Some(cancellation),
        )?;
        if !output.status.success() {
            // Conservatively report initialized if gitmodules exists and status failed.
            return Ok(true);
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.lines().any(|line| {
            let trimmed = line.trim_end();
            !trimmed.is_empty() && !trimmed.starts_with('-')
        }))
    }
}

struct RawDirtyEntry {
    path: PathBuf,
    orig_path: Option<PathBuf>,
    status_code: String,
    index_identity: String,
}

struct StatusSummary {
    unmerged: Vec<PathBuf>,
    staged_count: usize,
    unstaged_count: usize,
    untracked_count: usize,
    untracked_paths: Vec<PathBuf>,
    ignored_count: usize,
    ignored_sample: Vec<PathBuf>,
    ignored_all_paths: Vec<PathBuf>,
    ignored_paths_truncated: bool,
    raw_dirty_entries: Vec<RawDirtyEntry>,
    dirty_entries_truncated: bool,
    candidate_marker_paths: Vec<PathBuf>,
}

fn classify_operation(
    sequencer: SequencerState,
    has_merge_head: bool,
    has_unmerged: bool,
) -> FinishOperation {
    match sequencer {
        SequencerState::RebaseOrApply => FinishOperation::Rebase,
        SequencerState::CherryPick | SequencerState::Revert => FinishOperation::Other(sequencer),
        SequencerState::None => {
            if has_merge_head {
                FinishOperation::Merge
            } else if has_unmerged {
                FinishOperation::SquashMergeConflict
            } else {
                FinishOperation::None
            }
        }
    }
}

fn run_step_preserving_auth_error(
    cmd: std::process::Command,
    label: &str,
) -> Result<FinishStepCommandOutput> {
    let output = run_command_with_timeout(cmd, label, git_command_timeout(), None)?;
    if output.status.success() {
        return Ok(FinishStepCommandOutput::from_output(command_output(
            label, &output,
        )));
    }
    let cmd_out = command_output(label, &output);
    let classified = git_command_failed_error(label, output);
    if let ErrorKind::Git(failure) = classified.kind()
        && matches!(
            failure.auth_outcome(),
            Some(
                gitcomet_core::auth::AuthOutcome::NeedsInput(_)
                    | gitcomet_core::auth::AuthOutcome::Rejected(_)
            )
        )
    {
        return Err(classified);
    }
    Ok(FinishStepCommandOutput::from_output(cmd_out))
}

fn parse_merge_tree_conflict_paths(stdout: &[u8]) -> Result<Vec<PathBuf>> {
    // Format of `git merge-tree --write-tree -z --name-only` on conflict (exit 1):
    // <tree-oid>\0<conflict-path-1>\0<conflict-path-2>\0\0<informational-messages...>
    let mut fields = stdout.split(|b| *b == 0);
    // First token is the synthesized tree OID.
    let _tree_oid = fields.next();
    let mut paths = Vec::new();
    for field in fields {
        if field.is_empty() {
            // Empty NUL-delimited record marks the end of the conflicted file list.
            break;
        }
        let path = path_buf_from_git_bytes(field, "git merge-tree conflict path")?;
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn parse_porcelain_v2_status(bytes: &[u8]) -> Result<StatusSummary> {
    let mut summary = StatusSummary {
        unmerged: Vec::new(),
        staged_count: 0,
        unstaged_count: 0,
        untracked_count: 0,
        untracked_paths: Vec::new(),
        ignored_count: 0,
        ignored_sample: Vec::new(),
        ignored_all_paths: Vec::new(),
        ignored_paths_truncated: false,
        raw_dirty_entries: Vec::new(),
        dirty_entries_truncated: false,
        candidate_marker_paths: Vec::new(),
    };

    let mut fields = bytes.split(|b| *b == 0).peekable();
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        match record[0] {
            b'1' => {
                // `1 XY sub mH mI mW hH hI path` -> path starts after 8 spaces.
                if record.len() >= 4 {
                    if record[2] != b'.' {
                        summary.staged_count += 1;
                    }
                    if record[3] != b'.' {
                        summary.unstaged_count += 1;
                    }
                }
                if let Some((prefix, path_bytes)) = split_after_nth_space(record, 8) {
                    let path = path_buf_from_git_bytes(path_bytes, "git status path")?;
                    if summary.candidate_marker_paths.len() < MAX_CONFLICT_MARKER_SCAN_FILES {
                        summary.candidate_marker_paths.push(path.clone());
                    }
                    if summary.raw_dirty_entries.len() < MAX_DIRTY_VERIFY_ENTRIES {
                        let cols: Vec<&str> = std::str::from_utf8(prefix)
                            .unwrap_or_default()
                            .split(' ')
                            .collect();
                        let xy = cols.get(1).copied().unwrap_or("..");
                        let index_identity = format!(
                            "{}:{}:{}:{}",
                            cols.get(3).copied().unwrap_or(""),
                            cols.get(4).copied().unwrap_or(""),
                            cols.get(6).copied().unwrap_or(""),
                            cols.get(7).copied().unwrap_or(""),
                        );
                        summary.raw_dirty_entries.push(RawDirtyEntry {
                            path,
                            orig_path: None,
                            status_code: format!("1 {xy}"),
                            index_identity,
                        });
                    } else {
                        summary.dirty_entries_truncated = true;
                    }
                } else {
                    summary.dirty_entries_truncated = true;
                }
            }
            b'2' => {
                // `2 XY sub mH mI mW hH hI Xscore path\0origPath` -> consumes one extra NUL field.
                if record.len() >= 4 {
                    if record[2] != b'.' {
                        summary.staged_count += 1;
                    }
                    if record[3] != b'.' {
                        summary.unstaged_count += 1;
                    }
                }
                let orig_path_bytes = fields.next();
                if let Some((prefix, path_bytes)) = split_after_nth_space(record, 9)
                    && let Some(raw_orig) = orig_path_bytes.filter(|raw| !raw.is_empty())
                {
                    let path = path_buf_from_git_bytes(path_bytes, "git status rename path")?;
                    let orig_path =
                        Some(path_buf_from_git_bytes(raw_orig, "git status orig path")?);
                    if summary.candidate_marker_paths.len() < MAX_CONFLICT_MARKER_SCAN_FILES {
                        summary.candidate_marker_paths.push(path.clone());
                    }
                    if summary.raw_dirty_entries.len() < MAX_DIRTY_VERIFY_ENTRIES {
                        let cols: Vec<&str> = std::str::from_utf8(prefix)
                            .unwrap_or_default()
                            .split(' ')
                            .collect();
                        let xy = cols.get(1).copied().unwrap_or("..");
                        let index_identity = format!(
                            "{}:{}:{}:{}:{}",
                            cols.get(3).copied().unwrap_or(""),
                            cols.get(4).copied().unwrap_or(""),
                            cols.get(6).copied().unwrap_or(""),
                            cols.get(7).copied().unwrap_or(""),
                            cols.get(8).copied().unwrap_or(""),
                        );
                        summary.raw_dirty_entries.push(RawDirtyEntry {
                            path,
                            orig_path,
                            status_code: format!("2 {xy}"),
                            index_identity,
                        });
                    } else {
                        summary.dirty_entries_truncated = true;
                    }
                } else {
                    summary.dirty_entries_truncated = true;
                }
            }
            b'u' => {
                // `u XY sub m1 m2 m3 mW h1 h2 h3 path` -> path starts after 10 spaces.
                if let Some((prefix, path_bytes)) = split_after_nth_space(record, 10) {
                    let path = path_buf_from_git_bytes(path_bytes, "git status unmerged path")?;
                    if summary.candidate_marker_paths.len() < MAX_CONFLICT_MARKER_SCAN_FILES {
                        summary.candidate_marker_paths.push(path.clone());
                    }
                    summary.unmerged.push(path.clone());
                    if summary.raw_dirty_entries.len() < MAX_DIRTY_VERIFY_ENTRIES {
                        let cols: Vec<&str> = std::str::from_utf8(prefix)
                            .unwrap_or_default()
                            .split(' ')
                            .collect();
                        let xy = cols.get(1).copied().unwrap_or("UU");
                        let index_identity = format!(
                            "{}:{}:{}:{}:{}:{}",
                            cols.get(3).copied().unwrap_or(""),
                            cols.get(4).copied().unwrap_or(""),
                            cols.get(5).copied().unwrap_or(""),
                            cols.get(7).copied().unwrap_or(""),
                            cols.get(8).copied().unwrap_or(""),
                            cols.get(9).copied().unwrap_or(""),
                        );
                        summary.raw_dirty_entries.push(RawDirtyEntry {
                            path,
                            orig_path: None,
                            status_code: format!("u {xy}"),
                            index_identity,
                        });
                    } else {
                        summary.dirty_entries_truncated = true;
                    }
                } else {
                    summary.dirty_entries_truncated = true;
                }
            }
            b'?' => {
                summary.untracked_count += 1;
                if summary.untracked_paths.len() < MAX_UNTRACKED_PATHS
                    && let Some(path_bytes) = record.strip_prefix(b"? ")
                {
                    summary.untracked_paths.push(path_buf_from_git_bytes(
                        path_bytes,
                        "git status untracked path",
                    )?);
                }
            }
            b'!' => {
                summary.ignored_count += 1;
                if let Some(path_bytes) = record.strip_prefix(b"! ") {
                    let path = path_buf_from_git_bytes(path_bytes, "git status ignored path")?;
                    if summary.ignored_sample.len() < MAX_IGNORED_SAMPLE {
                        summary.ignored_sample.push(path.clone());
                    }
                    if summary.ignored_all_paths.len() < MAX_IGNORED_VERIFY_FILES {
                        summary.ignored_all_paths.push(path);
                    } else {
                        summary.ignored_paths_truncated = true;
                    }
                }
            }
            _ => {
                if record[0] != b'#' {
                    summary.dirty_entries_truncated = true;
                }
            }
        }
    }

    Ok(summary)
}

fn split_after_nth_space(bytes: &[u8], spaces: usize) -> Option<(&[u8], &[u8])> {
    let mut count = 0;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b' ' {
            count += 1;
            if count == spaces {
                return Some((&bytes[..idx], &bytes[idx + 1..]));
            }
        }
    }
    None
}

fn compute_ignored_fingerprint(
    workdir: &Path,
    ignored_paths: &[PathBuf],
    truncated: bool,
    cancellation: &CancellationToken,
) -> Result<IgnoredFilesFingerprint> {
    if truncated {
        return Ok(IgnoredFilesFingerprint::Unverifiable(
            "ignored file count exceeded verification budget".to_string(),
        ));
    }
    if ignored_paths.is_empty() {
        return Ok(IgnoredFilesFingerprint::empty_verified());
    }

    let mut roots: Vec<PathBuf> = ignored_paths
        .iter()
        .map(|p| {
            let s = p.to_string_lossy();
            let trimmed = s.trim_end_matches(['/', '\\']);
            PathBuf::from(trimmed)
        })
        .collect();
    roots.sort();
    roots.dedup();

    let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha256);
    let mut entry_count = 0usize;
    let mut bytes_hashed = 0u64;

    for root in &roots {
        if let Err(reason) = hash_ignored_tree(
            workdir,
            root,
            0,
            &mut entry_count,
            &mut bytes_hashed,
            &mut hasher,
            cancellation,
        )? {
            return Ok(IgnoredFilesFingerprint::Unverifiable(reason));
        }
    }

    let digest = hasher
        .try_finalize()
        .map_err(|err| Error::new(ErrorKind::Backend(err.to_string())))?
        .to_string();
    Ok(IgnoredFilesFingerprint::Verified {
        entry_count,
        digest,
    })
}

fn hash_ignored_tree(
    workdir: &Path,
    relative: &Path,
    depth: usize,
    entry_count: &mut usize,
    bytes_hashed: &mut u64,
    hasher: &mut gix::hash::Hasher,
    cancellation: &CancellationToken,
) -> Result<std::result::Result<(), String>> {
    cancellation.check_cancelled()?;
    if depth > MAX_IGNORED_VERIFY_DEPTH {
        return Ok(Err("ignored directory nesting exceeded depth limit".into()));
    }
    *entry_count = entry_count.saturating_add(1);
    if *entry_count > MAX_IGNORED_VERIFY_FILES {
        return Ok(Err("ignored file count exceeded verification budget".into()));
    }

    let full_path = workdir.join(relative);
    let metadata = match std::fs::symlink_metadata(&full_path) {
        Ok(meta) => meta,
        Err(err) => {
            return Ok(Err(format!(
                "could not inspect ignored path '{}': {err}",
                relative.display()
            )));
        }
    };

    let rel_bytes = relative.as_os_str().as_encoded_bytes();
    hasher.update(&(rel_bytes.len() as u64).to_le_bytes());
    hasher.update(rel_bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        hasher.update(&metadata.permissions().mode().to_le_bytes());
    }
    #[cfg(not(unix))]
    hasher.update(&[u8::from(metadata.permissions().readonly())]);

    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        hasher.update(b"symlink");
        match std::fs::read_link(&full_path) {
            Ok(target) => {
                let bytes = target.as_os_str().as_encoded_bytes();
                hasher.update(&(bytes.len() as u64).to_le_bytes());
                hasher.update(bytes);
                return Ok(Ok(()));
            }
            Err(err) => {
                return Ok(Err(format!(
                    "could not read ignored symlink '{}': {err}",
                    relative.display()
                )));
            }
        }
    }

    if file_type.is_file() {
        hasher.update(b"file");
        let len = metadata.len();
        hasher.update(&len.to_le_bytes());
        if bytes_hashed.saturating_add(len) > MAX_IGNORED_VERIFY_BYTES {
            return Ok(Err(
                "ignored file total size exceeded verification budget".into()
            ));
        }
        let mut file = match File::open(&full_path) {
            Ok(f) => f,
            Err(err) => {
                return Ok(Err(format!(
                    "could not read ignored file '{}': {err}",
                    relative.display()
                )));
            }
        };
        let mut buf = [0u8; 64 * 1024];
        loop {
            cancellation.check_cancelled()?;
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    *bytes_hashed = bytes_hashed.saturating_add(n as u64);
                    if *bytes_hashed > MAX_IGNORED_VERIFY_BYTES {
                        return Ok(Err(
                            "ignored file total size exceeded verification budget".into()
                        ));
                    }
                    hasher.update(&buf[..n]);
                }
                Err(err) => {
                    return Ok(Err(format!(
                        "failed reading ignored file '{}': {err}",
                        relative.display()
                    )));
                }
            }
        }
        return Ok(Ok(()));
    }

    if file_type.is_dir() {
        hasher.update(b"dir");
        let read_dir = match std::fs::read_dir(&full_path) {
            Ok(rd) => rd,
            Err(err) => {
                return Ok(Err(format!(
                    "could not read ignored directory '{}': {err}",
                    relative.display()
                )));
            }
        };
        let mut children = Vec::new();
        for entry in read_dir {
            match entry {
                Ok(e) => children.push(e.file_name()),
                Err(err) => {
                    return Ok(Err(format!(
                        "failed enumerating ignored directory '{}': {err}",
                        relative.display()
                    )));
                }
            }
        }
        children.sort();
        for child_name in children {
            if let Err(reason) = hash_ignored_tree(
                workdir,
                &relative.join(child_name),
                depth + 1,
                entry_count,
                bytes_hashed,
                hasher,
                cancellation,
            )? {
                return Ok(Err(reason));
            }
        }
        return Ok(Ok(()));
    }

    Ok(Err(format!(
        "ignored path '{}' is not a regular file, symlink, or directory",
        relative.display()
    )))
}

fn build_dirty_entries(
    workdir: &Path,
    raw: &[RawDirtyEntry],
    truncated: bool,
    cancellation: &CancellationToken,
) -> Result<Option<Vec<WorktreeEntryDigest>>> {
    if truncated {
        return Ok(None);
    }
    if raw.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut entries = Vec::with_capacity(raw.len());
    let mut bytes_hashed = 0u64;
    for item in raw {
        cancellation.check_cancelled()?;
        let worktree_digest =
            match hash_single_worktree_entry(&workdir.join(&item.path), &mut bytes_hashed)? {
                SingleEntryHash::Missing => None,
                SingleEntryHash::Digest(d) => Some(d),
                SingleEntryHash::Unverifiable => return Ok(None),
            };
        entries.push(WorktreeEntryDigest {
            path: item.path.clone(),
            orig_path: item.orig_path.clone(),
            status_code: item.status_code.clone(),
            index_identity: item.index_identity.clone(),
            worktree_digest,
        });
    }
    entries.sort_by(|a, b| (&a.path, &a.orig_path).cmp(&(&b.path, &b.orig_path)));
    Ok(Some(entries))
}

enum SingleEntryHash {
    Missing,
    Digest(String),
    Unverifiable,
}

fn hash_single_worktree_entry(path: &Path, bytes_hashed: &mut u64) -> Result<SingleEntryHash> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SingleEntryHash::Missing);
        }
        Err(_) => return Ok(SingleEntryHash::Unverifiable),
    };
    let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha256);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        hasher.update(&metadata.permissions().mode().to_le_bytes());
    }
    #[cfg(not(unix))]
    hasher.update(&[u8::from(metadata.permissions().readonly())]);

    let ft = metadata.file_type();
    if ft.is_symlink() {
        hasher.update(b"symlink");
        let Ok(target) = std::fs::read_link(path) else {
            return Ok(SingleEntryHash::Unverifiable);
        };
        hasher.update(target.as_os_str().as_encoded_bytes());
    } else if ft.is_file() {
        hasher.update(b"file");
        let len = metadata.len();
        if bytes_hashed.saturating_add(len) > MAX_DIRTY_VERIFY_BYTES {
            return Ok(SingleEntryHash::Unverifiable);
        }
        hasher.update(&len.to_le_bytes());
        let Ok(mut file) = File::open(path) else {
            return Ok(SingleEntryHash::Unverifiable);
        };
        let mut buf = [0u8; 64 * 1024];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    *bytes_hashed = bytes_hashed.saturating_add(n as u64);
                    if *bytes_hashed > MAX_DIRTY_VERIFY_BYTES {
                        return Ok(SingleEntryHash::Unverifiable);
                    }
                    hasher.update(&buf[..n]);
                }
                Err(_) => return Ok(SingleEntryHash::Unverifiable),
            }
        }
    } else if ft.is_dir() {
        // Gitlink / submodule directory; cannot safely prove unmodified without deep check.
        return Ok(SingleEntryHash::Unverifiable);
    } else {
        return Ok(SingleEntryHash::Unverifiable);
    }

    let digest = hasher
        .try_finalize()
        .map_err(|err| Error::new(ErrorKind::Backend(err.to_string())))?
        .to_string();
    Ok(SingleEntryHash::Digest(digest))
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

fn scan_conflict_markers(workdir: &Path, paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut flagged = Vec::new();
    for rel in paths {
        let full = workdir.join(rel);
        let Ok(file) = File::open(&full) else {
            continue;
        };
        let reader = BufReader::new(file);
        if matches!(
            reader_has_conflict_markers(reader, MAX_CONFLICT_MARKER_SCAN_BYTES),
            Ok(true)
        ) {
            flagged.push(rel.clone());
        }
    }
    flagged
}

/// First entry under `dir` (relative to `workdir`) that an abort may not delete.
///
/// Walks without following symlinks, so ignored and untracked entries are seen too. A file or
/// symlink is allowed only when it is one of `owned_removals`; a directory only when it holds
/// some owned removal (an empty or unowned directory is user data). Unreadable directories count
/// as unprovable and are returned as the blocking entry.
pub(super) fn first_unowned_entry_in_dir(
    workdir: &Path,
    dir: &Path,
    owned_removals: &BTreeSet<&Path>,
) -> Option<PathBuf> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(workdir.join(&current)) else {
            return Some(current);
        };
        for entry in entries {
            let Ok(entry) = entry else {
                return Some(current);
            };
            let relative = current.join(entry.file_name());
            let Ok(file_type) = entry.file_type() else {
                return Some(relative);
            };
            let allowed = if file_type.is_dir() {
                owned_removals
                    .iter()
                    .any(|owned| owned.starts_with(&relative) && *owned != relative)
            } else {
                owned_removals.contains(relative.as_path())
            };
            if !allowed {
                return Some(relative);
            }
            if file_type.is_dir() {
                pending.push(relative);
            }
        }
    }
    None
}
