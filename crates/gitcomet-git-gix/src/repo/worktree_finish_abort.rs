//! Guarded `git merge --abort` / `git rebase --abort` for Worktree Finish.
//!
//! Both aborts restore a commit's tree over the working tree, and Git treats ignored files as
//! expendable while doing so: a directory standing where the restored tree has a file is removed
//! with everything in it. These wrappers prove first that the restore only replaces entries the
//! index tracks, and otherwise refuse before Git writes anything.

use super::GixRepo;
use super::worktree_finish::{NO_COLOR_CONFIG, first_unowned_entry_in_dir};
use crate::util::{
    path_buf_from_git_bytes, run_git_capture_bytes_cancellable, run_git_with_output,
    validate_hex_commit_id,
};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, CommandOutput, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Where `git rebase` keeps `orig-head`: the merge backend first, then the apply backend.
const REBASE_STATE_DIRS: [&str; 2] = ["rebase-merge", "rebase-apply"];

fn refusal(operation: &str, reason: String) -> Error {
    Error::new(ErrorKind::Backend(format!(
        "{operation} abort refused: {reason}; preserving index, working tree and conflict state for manual recovery."
    )))
}

impl GixRepo {
    pub(super) fn abort_merge_checked_impl(&self) -> Result<CommandOutput> {
        self.refuse_unprovable_abort_restore("Merge", "HEAD")?;
        self.merge_abort_with_output_impl()
    }

    /// Runs plain `git rebase --abort`: the generic abort's cherry-pick, revert and `git am`
    /// fallbacks restore other commits, which the proof below does not cover.
    pub(super) fn abort_rebase_checked_impl(&self) -> Result<CommandOutput> {
        let original_head = self.rebase_original_head()?;
        self.refuse_unprovable_abort_restore("Rebase", &original_head)?;
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .arg("rebase")
            .arg("--abort");
        run_git_with_output(cmd, "git rebase --abort")
    }

    /// The commit `git rebase --abort` restores, read from this worktree's rebase state.
    fn rebase_original_head(&self) -> Result<String> {
        let git_dir = self.repo().path().to_path_buf();
        for state_dir in REBASE_STATE_DIRS {
            let Ok(recorded) = std::fs::read_to_string(git_dir.join(state_dir).join("orig-head"))
            else {
                continue;
            };
            let original_head = recorded.trim().to_string();
            validate_hex_commit_id(&CommitId(original_head.as_str().into())).map_err(|_| {
                refusal(
                    "Rebase",
                    format!("'{state_dir}/orig-head' does not hold a commit id"),
                )
            })?;
            return Ok(original_head);
        }
        Err(refusal(
            "Rebase",
            "could not read the commit the rebase started from".to_string(),
        ))
    }

    /// Fails unless everything on disk that restoring `restore_commit` would replace is tracked
    /// by the index: a directory where the commit has a file may hold only tracked entries, and
    /// a file standing on a restored path or on one of its parent directories must be tracked.
    ///
    /// Ignored and untracked entries (and empty directories) are therefore never deleted. Edits
    /// to tracked files are not judged here; the caller compares them with its conflict baseline.
    fn refuse_unprovable_abort_restore(&self, operation: &str, restore_commit: &str) -> Result<()> {
        let restored_files = self.restore_tree_file_paths(restore_commit)?;
        let tracked = self.index_paths()?;
        let workdir = &self.spec.workdir;
        let mut inspected_parents: BTreeSet<&Path> = BTreeSet::new();
        for path in &restored_files {
            let blocker = untracked_entry_at(workdir, path, &tracked)
                .or_else(|| untracked_parent_of(workdir, path, &tracked, &mut inspected_parents));
            if let Some(reason) = blocker {
                return Err(refusal(operation, reason));
            }
        }
        Ok(())
    }

    /// Every file or symlink path in `commit`'s tree. Submodule entries are left out: Git never
    /// replaces a submodule's directory while restoring its parent.
    fn restore_tree_file_paths(&self, commit: &str) -> Result<Vec<PathBuf>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .arg("ls-tree")
            .arg("-r")
            .arg("-z")
            .arg("--full-tree")
            .arg(commit);
        let listing = run_git_capture_bytes_cancellable(
            cmd,
            "git ls-tree -r -z --full-tree <restore commit>",
            &CancellationToken::new(),
        )?;
        let mut paths = Vec::new();
        for record in listing.split(|byte| *byte == 0) {
            if record.is_empty() {
                continue;
            }
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                return Err(Error::new(ErrorKind::Backend(
                    "unexpected `git ls-tree` record while checking an abort restore".to_string(),
                )));
            };
            let is_submodule = record[..tab]
                .split(|byte| *byte == b' ')
                .nth(1)
                .is_some_and(|object_type| object_type == b"commit");
            if !is_submodule {
                paths.push(path_buf_from_git_bytes(
                    &record[tab + 1..],
                    "git ls-tree restore path",
                )?);
            }
        }
        Ok(paths)
    }

    /// Every path the index tracks, at any stage.
    fn index_paths(&self) -> Result<BTreeSet<PathBuf>> {
        let mut cmd = self.git_workdir_cmd();
        cmd.args(NO_COLOR_CONFIG)
            .arg("--no-pager")
            .arg("ls-files")
            .arg("-z")
            .arg("--full-name");
        let listing = run_git_capture_bytes_cancellable(
            cmd,
            "git ls-files -z --full-name",
            &CancellationToken::new(),
        )?;
        listing
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .map(|record| path_buf_from_git_bytes(record, "git ls-files index path"))
            .collect()
    }
}

/// Why restoring the file `path` would destroy untracked data occupying that very path.
fn untracked_entry_at(workdir: &Path, path: &Path, tracked: &BTreeSet<PathBuf>) -> Option<String> {
    match std::fs::symlink_metadata(workdir.join(path)) {
        Ok(meta) if meta.file_type().is_dir() => {
            let tracked_below: BTreeSet<&Path> = tracked
                .iter()
                .map(PathBuf::as_path)
                .filter(|tracked_path| tracked_path.starts_with(path))
                .collect();
            first_unowned_entry_in_dir(workdir, path, &tracked_below).map(|unowned| {
                format!(
                    "restoring '{}' as a file would delete '{}', which Git does not track",
                    path.display(),
                    unowned.display()
                )
            })
        }
        Ok(_) if !tracked.contains(path) => Some(format!(
            "restoring '{}' would overwrite a file Git does not track",
            path.display()
        )),
        Ok(_) => None,
        // Nothing stands on the path itself; a file on a parent directory is checked separately.
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            None
        }
        Err(err) => Some(format!("could not inspect '{}' ({err})", path.display())),
    }
}

/// Why restoring `path` would replace an untracked file standing where one of its parent
/// directories has to be. `inspected_parents` skips directories earlier paths already covered.
fn untracked_parent_of<'a>(
    workdir: &Path,
    path: &'a Path,
    tracked: &BTreeSet<PathBuf>,
    inspected_parents: &mut BTreeSet<&'a Path>,
) -> Option<String> {
    for parent in path.ancestors().skip(1) {
        // An inspected parent already covered its own parents.
        if parent.as_os_str().is_empty() || !inspected_parents.insert(parent) {
            break;
        }
        if let Ok(meta) = std::fs::symlink_metadata(workdir.join(parent))
            && !meta.file_type().is_dir()
            && !tracked.contains(parent)
        {
            return Some(format!(
                "restoring '{}' would replace '{}', which Git does not track",
                path.display(),
                parent.display()
            ));
        }
    }
    None
}
