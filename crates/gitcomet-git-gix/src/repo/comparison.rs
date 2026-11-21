//! Comparisons shared by History, commit ranges, submodules, and hosted
//! views: which files differ between two points, with rename sources, object
//! ids, and modes; merge bases; ancestry.
//!
//! Commit-to-commit comparisons diff trees in-process; a working-tree tip
//! has no tree object, so those use `git diff --raw` against the worktree,
//! which is also what the unified diff in the main pane shows.

use super::GixRepo;
use super::log::diff_range_files;
use super::patch::write_pathspec_file;
use crate::util::{
    git_workdir_cmd_for, path_buf_from_git_bytes, push_literal_pathspec_file,
    push_read_only_diff_flags, run_git_capture_bytes_cancellable,
};
use gitcomet_core::domain::{CommitFileChange, CommitId, FileMode, FileStatusKind, ObjectHash};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{
    CancellationToken, Comparison, ComparisonBase, ComparisonOptions, Result,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) type NumstatLineCounts = (Option<u32>, Option<u32>);
/// Lookup only (never iterated in order), so the unseeded hash map suffices.
pub(super) type NumstatCounts = rustc_hash::FxHashMap<PathBuf, NumstatLineCounts>;

/// One entry of a `git diff --raw` listing.
pub(super) struct RangeStatusChange {
    pub(super) path: PathBuf,
    pub(super) old_path: Option<PathBuf>,
    pub(super) kind: FileStatusKind,
    /// A gitlink on either side: a submodule pointer rather than a file.
    pub(super) is_submodule: bool,
    pub(super) old_id: Option<ObjectHash>,
    pub(super) new_id: Option<ObjectHash>,
    pub(super) old_mode: Option<FileMode>,
    pub(super) new_mode: Option<FileMode>,
}

impl RangeStatusChange {
    /// numstat's counts for this change. A gitlink's numstat counts its
    /// "Subproject commit" line, not content, so a submodule has none. The
    /// numstat runs here ignore submodules, which leaves out pointer moves
    /// but not a gitlink replaced by a file, so the row decides.
    pub(super) fn line_counts(&self, counts: &NumstatCounts) -> NumstatLineCounts {
        if self.is_submodule {
            return (None, None);
        }
        counts.get(&self.path).cloned().unwrap_or((None, None))
    }
}

/// Git's tree entry mode for a gitlink (a submodule pointer).
const GITLINK_ENTRY_MODE: &[u8] = b"160000";

pub(super) fn parse_numstat_field(field: &[u8]) -> Option<u32> {
    if field == b"-" {
        return None;
    }
    std::str::from_utf8(field).ok()?.parse::<u32>().ok()
}

fn next_non_empty_nul_field<'a, I>(fields: &mut I) -> Option<&'a [u8]>
where
    I: Iterator<Item = &'a [u8]>,
{
    fields.find(|field| !field.is_empty())
}

/// `git diff <from> [<to>]` in one of git's machine formats; omitting `to`
/// compares against the working tree. A comparison index also isolates any
/// objects Git writes while preparing or reading the comparison.
fn range_diff_command(
    workdir: &Path,
    format: &[&str],
    from: &CommitId,
    to: Option<&CommitId>,
    index: Option<&ComparisonIndex>,
) -> Command {
    let mut command = git_workdir_cmd_for(workdir);
    if let Some(index) = index {
        index.configure(&mut command);
    }
    push_read_only_diff_flags(&mut command);
    command
        .arg("diff")
        .args(format)
        .arg("-z")
        .arg("--find-renames")
        .arg(from.as_ref());
    if let Some(to) = to {
        command.arg(to.as_ref());
    }
    command
}

/// `--raw` rather than `--name-status` because the entry modes are the only
/// thing in a CLI diff that identifies a submodule pointer, and callers that
/// build `CommitFileChange` have to flag those the same way the gix tree-diff
/// path does.
pub(super) fn git_range_status_changes(
    workdir: &Path,
    from: &CommitId,
    to: Option<&CommitId>,
    index: Option<&ComparisonIndex>,
    cancellation: &CancellationToken,
) -> Result<Vec<RangeStatusChange>> {
    let command = range_diff_command(workdir, &["--raw", "--no-abbrev"], from, to, index);
    let label = "git diff --raw -z --find-renames";
    let output = run_git_capture_bytes_cancellable(command, label, cancellation)?;
    parse_raw_changes(&output, cancellation)
}

/// Parses `git diff --raw -z` records:
/// `:<srcmode> <dstmode> <srcsha> <dstsha> <status>\0<path>\0`, with renames
/// and copies adding the destination as a second path.
fn parse_raw_changes(
    output: &[u8],
    cancellation: &CancellationToken,
) -> Result<Vec<RangeStatusChange>> {
    let mut fields = output.split(|byte| *byte == 0);
    let mut changes = Vec::new();
    while let Some(header) = next_non_empty_nul_field(&mut fields) {
        cancellation.check_cancelled()?;
        let Some(header) = header.strip_prefix(b":") else {
            continue;
        };
        let tokens: Vec<&[u8]> = header
            .split(|byte| *byte == b' ')
            .filter(|token| !token.is_empty())
            .collect();
        let [src_mode, dst_mode, src_id, dst_id, status_field] = tokens[..] else {
            continue;
        };
        let Some(status_code) = status_field.first().copied() else {
            continue;
        };
        let kind = match status_code {
            b'A' | b'C' => FileStatusKind::Added,
            b'D' => FileStatusKind::Deleted,
            b'R' => FileStatusKind::Renamed,
            b'U' => FileStatusKind::Conflicted,
            _ => FileStatusKind::Modified,
        };

        let (old_path_bytes, path_bytes) = if matches!(status_code, b'R' | b'C') {
            let old = next_non_empty_nul_field(&mut fields);
            (
                old,
                next_non_empty_nul_field(&mut fields).unwrap_or_default(),
            )
        } else {
            (
                None,
                next_non_empty_nul_field(&mut fields).unwrap_or_default(),
            )
        };
        if path_bytes.is_empty() {
            continue;
        }
        let old_path = old_path_bytes
            .map(|bytes| path_buf_from_git_bytes(bytes, "git diff --raw source path"))
            .transpose()?;

        let hex = |id: &[u8]| std::str::from_utf8(id).ok().and_then(ObjectHash::from_hex);
        changes.push(RangeStatusChange {
            path: path_buf_from_git_bytes(path_bytes, "git diff --raw path")?,
            old_path,
            kind,
            is_submodule: src_mode == GITLINK_ENTRY_MODE || dst_mode == GITLINK_ENTRY_MODE,
            old_id: hex(src_id),
            new_id: hex(dst_id),
            old_mode: FileMode::from_octal(src_mode),
            new_mode: FileMode::from_octal(dst_mode),
        });
    }
    Ok(changes)
}

pub(super) fn git_range_numstat_counts(
    workdir: &Path,
    from: &CommitId,
    to: Option<&CommitId>,
    index: Option<&ComparisonIndex>,
    cancellation: &CancellationToken,
) -> Result<NumstatCounts> {
    // Gitlink counts are dropped per row, so skip the `git status` git would
    // otherwise run in every populated submodule against the working tree.
    let command = range_diff_command(
        workdir,
        &["--numstat", "--ignore-submodules=all"],
        from,
        to,
        index,
    );
    let label = "git diff --numstat --ignore-submodules=all -z --find-renames";
    let output = run_git_capture_bytes_cancellable(command, label, cancellation)?;

    let mut counts = NumstatCounts::default();
    let mut fields = output.split(|byte| *byte == 0);
    while let Some(record) = next_non_empty_nul_field(&mut fields) {
        cancellation.check_cancelled()?;
        let mut columns = record.splitn(3, |byte| *byte == b'\t');
        let additions = parse_numstat_field(columns.next().unwrap_or_default());
        let deletions = parse_numstat_field(columns.next().unwrap_or_default());
        let path_field = columns.next().unwrap_or_default();
        let path_bytes = if path_field.is_empty() {
            let _old_path = next_non_empty_nul_field(&mut fields);
            next_non_empty_nul_field(&mut fields).unwrap_or_default()
        } else {
            path_field
        };
        if path_bytes.is_empty() {
            continue;
        }
        counts.insert(
            path_buf_from_git_bytes(path_bytes, "git diff --numstat path")?,
            (additions, deletions),
        );
    }

    Ok(counts)
}

/// Untracked, not ignored files in the working tree.
fn untracked_paths(workdir: &Path, cancellation: &CancellationToken) -> Result<Vec<PathBuf>> {
    let mut command = git_workdir_cmd_for(workdir);
    command
        .arg("--no-optional-locks")
        .arg("ls-files")
        .arg("--others")
        .arg("--exclude-standard")
        .arg("-z");
    let output = run_git_capture_bytes_cancellable(
        command,
        "git ls-files --others --exclude-standard -z",
        cancellation,
    )?;
    output
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| path_buf_from_git_bytes(field, "git ls-files path"))
        .collect()
}

/// A temporary index and its private object store. Git can read the
/// repository's existing objects through alternates, but intent-to-add can
/// write its empty blob only to the private store.
pub(super) struct ComparisonIndex {
    directory: tempfile::TempDir,
    alternates: OsString,
}

impl ComparisonIndex {
    pub(super) fn configure(&self, command: &mut Command) {
        command
            .env("GIT_INDEX_FILE", self.directory.path().join("index"))
            .env(
                "GIT_OBJECT_DIRECTORY",
                self.directory.path().join("objects"),
            )
            .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &self.alternates);
    }
}

fn comparison_alternates(objects: &Path) -> Result<OsString> {
    let bytes = gix::path::into_bstr(objects).map_err(|err| {
        Error::new(ErrorKind::Backend(format!(
            "encode Git object directory: {err}"
        )))
    })?;
    let bytes = gix::path::to_unix_separators_on_windows(bytes);
    let quoted = gix::quote::ansi_c::quote(&bytes);
    let quoted = std::str::from_utf8(quoted.as_ref()).expect("Git C-style quotes are ASCII");
    // The quote helper can leave ASCII paths unchanged, but an alternate entry
    // may contain Git's platform-specific path-list delimiter.
    let mut alternates = if quoted.starts_with('"') {
        OsString::from(quoted)
    } else {
        OsString::from(format!("\"{quoted}\""))
    };
    if let Some(existing) =
        std::env::var_os("GIT_ALTERNATE_OBJECT_DIRECTORIES").filter(|value| !value.is_empty())
    {
        alternates.push(if cfg!(windows) { ";" } else { ":" });
        alternates.push(existing);
    }
    Ok(alternates)
}

/// Add untracked paths as intent-to-add entries without changing the source
/// index or writing objects into the repository. Keep the result alive until
/// the last diff command has finished.
pub(super) fn intent_to_add_index(
    workdir: &Path,
    repo: &gix::Repository,
    paths: &[PathBuf],
    cancellation: &CancellationToken,
) -> Result<ComparisonIndex> {
    cancellation.check_cancelled()?;
    let path_bytes = paths
        .iter()
        .map(|path| {
            let bytes = gix::path::into_bstr(path.as_path()).map_err(|err| {
                Error::new(ErrorKind::Backend(format!("encode untracked path: {err}")))
            })?;
            Ok(gix::path::to_unix_separators_on_windows(bytes).to_vec())
        })
        .collect::<Result<Vec<_>>>()?;
    let pathspec = write_pathspec_file(path_bytes.iter().map(Vec::as_slice))?;
    let objects = std::env::var_os("GIT_OBJECT_DIRECTORY")
        .filter(|directory| !directory.is_empty())
        .map(|directory| workdir.join(directory))
        .unwrap_or_else(|| repo.objects.store_ref().path().to_path_buf());
    let index = ComparisonIndex {
        directory: tempfile::tempdir()?,
        alternates: comparison_alternates(&objects)?,
    };
    std::fs::create_dir(index.directory.path().join("objects"))?;
    match std::fs::copy(repo.index_path(), index.directory.path().join("index")) {
        Ok(_) => {}
        // A repository can have a tree to compare against but no index yet.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    let mut command = git_workdir_cmd_for(workdir);
    index.configure(&mut command);
    command.args(["-c", "core.splitIndex=false", "add", "--intent-to-add"]);
    push_literal_pathspec_file(&mut command, pathspec.path());
    run_git_capture_bytes_cancellable(command, "git add --intent-to-add", cancellation)?;
    Ok(index)
}

/// What `git diff <from>` lists against the live working tree, with
/// numstat's counts. Untracked files, listed only when asked for, have none.
///
/// `git diff` does not see untracked files, so a file moved without `git mv`
/// would list as deleted, its new copy as a separate untracked file. With the
/// untracked files as intent-to-add entries git pairs the two into a rename.
fn git_worktree_changes(
    workdir: &Path,
    repo: &gix::Repository,
    from: &CommitId,
    include_untracked: bool,
    cancellation: &CancellationToken,
) -> Result<Vec<CommitFileChange>> {
    let untracked = if include_untracked {
        untracked_paths(workdir, cancellation)?
    } else {
        Vec::new()
    };
    let index = if untracked.is_empty() {
        None
    } else {
        Some(intent_to_add_index(
            workdir,
            repo,
            &untracked,
            cancellation,
        )?)
    };
    let status_changes =
        git_range_status_changes(workdir, from, None, index.as_ref(), cancellation)?;
    let counts = git_range_numstat_counts(workdir, from, None, index.as_ref(), cancellation)?;
    let untracked_set: rustc_hash::FxHashSet<&Path> =
        untracked.iter().map(PathBuf::as_path).collect();
    let files = status_changes
        .into_iter()
        // Without the index refresh `--raw` also lists a file whose stat
        // moved but not its content; numstat reads the content and leaves
        // it out, while every real change (mode-only, binary) is in it.
        .filter(|change| {
            change.kind != FileStatusKind::Modified
                || change.is_submodule
                || counts.contains_key(&change.path)
        })
        .map(|change| {
            // Raw/numstat from the temporary index decide whether a retained
            // path differs from `from`. Only genuine additions are untracked.
            if change.kind == FileStatusKind::Added && untracked_set.contains(change.path.as_path())
            {
                // Let the worktree stats pass preserve caps, symlinks and
                // clean-filter handling, as in the untracked status lane.
                return CommitFileChange::new(change.path, FileStatusKind::Untracked);
            }
            let line_stats = change.line_counts(&counts).into();
            CommitFileChange::new(change.path, change.kind)
                .with_submodule(change.is_submodule)
                .with_line_stats(line_stats)
                .with_old_path(change.old_path)
                .with_ids(change.old_id, change.new_id)
                .with_modes(change.old_mode, change.new_mode)
        })
        .collect();
    Ok(files)
}

impl GixRepo {
    /// The files that differ between commit `from` and the live working tree
    /// (`git diff <from>`), with their line stats. Untracked files are listed
    /// only when asked for.
    pub(super) fn commit_to_worktree_files(
        &self,
        from: &CommitId,
        include_untracked: bool,
        cancellation: &CancellationToken,
    ) -> Result<Vec<CommitFileChange>> {
        let mut files = git_worktree_changes(
            &self.spec.workdir,
            &self.repo(),
            from,
            include_untracked,
            cancellation,
        )?;
        self.add_worktree_line_stats(&mut files, cancellation)?;
        Ok(files)
    }

    pub(super) fn compare_files_impl(
        &self,
        from: &CommitId,
        to: Option<&CommitId>,
        options: &ComparisonOptions,
        cancellation: &CancellationToken,
    ) -> Result<Comparison> {
        let base = match options.base {
            ComparisonBase::Direct => from.clone(),
            ComparisonBase::MergeBase => {
                let head = match to {
                    Some(to) => to.clone(),
                    None => super::history::gix_head_id_or_none(&self.repo())?
                        .map(|id| CommitId(id.to_string().into()))
                        .ok_or_else(|| {
                            Error::new(ErrorKind::Backend(
                                "a merge-base comparison with the working tree needs a HEAD commit"
                                    .to_string(),
                            ))
                        })?,
                };
                self.merge_base_impl(from, &head)?.ok_or_else(|| {
                    Error::new(ErrorKind::Backend(format!(
                        "{from} and {head} have no merge base"
                    )))
                })?
            }
        };
        cancellation.check_cancelled()?;
        let files = match to {
            Some(to) => diff_range_files(self, &self.repo(), &base, to)?,
            None => {
                self.commit_to_worktree_files(&base, options.include_untracked, cancellation)?
            }
        };
        cancellation.check_cancelled()?;
        Ok(Comparison::new(base, files))
    }

    fn merge_base_impl(&self, a: &CommitId, b: &CommitId) -> Result<Option<CommitId>> {
        let repo = self.repo();
        let a = resolve_commit(&repo, a)?;
        let b = resolve_commit(&repo, b)?;
        Ok(merge_base_ids(&repo, a, b)?.map(|id| CommitId(id.to_string().into())))
    }
}

/// The best merge base, or `None` for unrelated histories. Through the
/// commit-graph when the repository has one enabled.
fn merge_base_ids(
    repo: &gix::Repository,
    a: gix::ObjectId,
    b: gix::ObjectId,
) -> Result<Option<gix::ObjectId>> {
    let cache = repo
        .commit_graph_if_enabled()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix commit-graph: {e}"))))?;
    let mut graph = repo.revision_graph(cache.as_ref());
    gix::revision::plumbing::merge_base(a, &[b], &mut graph)
        .map(|bases| bases.map(|bases| *bases.first()))
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix merge-base: {e}"))))
}

fn resolve_commit(repo: &gix::Repository, id: &CommitId) -> Result<gix::ObjectId> {
    let spec = id.as_ref();
    crate::refs::resolve_required(repo, spec)
        .map_err(|e| {
            Error::new(ErrorKind::Backend(format!("rev-parse {spec}: {e}"))).with_source(e)
        })?
        .object()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("object {spec}: {e}"))))?
        .peel_to_commit()
        .map(|commit| commit.id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("{spec} is not a commit: {e}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A comparison against the working tree is a read: git must not write
    /// refreshed stats back to the index (the watcher would report an index
    /// change and reload every worktree view), and a file whose stat moved
    /// without its content is not a change.
    #[test]
    fn comparison_against_the_worktree_does_not_write_the_index() {
        use crate::test_support::{git_success, init_test_repo, open_repo, write_file};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "touched.txt", "unchanged content\n");
        write_file(dir, "edited.txt", "one\ntwo\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        // Stale stats: git would re-hash both and record the new stats.
        let stale =
            filetime::FileTime::from_unix_time(filetime::FileTime::now().unix_seconds() - 120, 0);
        write_file(dir, "edited.txt", "one\n2\n");
        for name in ["touched.txt", "edited.txt"] {
            filetime::set_file_mtime(dir.join(name), stale).unwrap();
        }
        let index = dir.join(".git/index");
        let before = std::fs::read(&index).unwrap();

        let repo = open_repo(dir);
        let head = super::super::history::gix_head_id_or_none(&repo.repo())
            .unwrap()
            .unwrap();
        let files = repo
            .commit_to_worktree_files(
                &CommitId(head.to_string().into()),
                false,
                &CancellationToken::new(),
            )
            .unwrap();
        let listed: Vec<_> = files
            .iter()
            .map(|file| {
                (
                    file.path.as_path(),
                    file.kind,
                    file.line_stats.additions,
                    file.line_stats.deletions,
                )
            })
            .collect();
        assert_eq!(
            listed,
            vec![(
                Path::new("edited.txt"),
                FileStatusKind::Modified,
                Some(1),
                Some(1)
            )]
        );
        assert_eq!(
            std::fs::read(&index).unwrap(),
            before,
            "the comparison refreshed index stat metadata"
        );
    }

    #[test]
    fn intent_to_add_comparison_does_not_reappend_index_removed_paths() {
        use crate::test_support::{git_success, init_test_repo, open_repo, write_file};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "old.txt", "original\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        git_success(dir, &["rm", "--cached", "old.txt"]);
        let index = dir.join(".git/index");
        let before = std::fs::read(&index).unwrap();
        let repo = open_repo(dir);
        let from = CommitId("HEAD".into());
        let cancellation = CancellationToken::new();
        assert!(
            repo.commit_to_worktree_files(&from, true, &cancellation)
                .unwrap()
                .is_empty(),
            "a retained file identical to HEAD is not a comparison change"
        );

        write_file(dir, "old.txt", "changed\n");
        let files = repo
            .commit_to_worktree_files(&from, true, &cancellation)
            .unwrap();
        assert_eq!(
            files.len(),
            1,
            "one path must not be both modified and untracked"
        );
        assert_eq!(files[0].path, Path::new("old.txt"));
        assert_eq!(files[0].kind, FileStatusKind::Modified);
        assert_eq!(files[0].line_stats.additions, Some(1));
        assert_eq!(files[0].line_stats.deletions, Some(1));

        write_file(dir, "new.txt", "genuinely new\n");
        write_file(dir, "empty.txt", "");
        let files = repo
            .commit_to_worktree_files(&from, true, &cancellation)
            .unwrap();
        assert_eq!(files.len(), 3);
        for name in ["new.txt", "empty.txt"] {
            let file = files
                .iter()
                .find(|file| file.path == Path::new(name))
                .unwrap();
            assert_eq!(file.kind, FileStatusKind::Untracked, "{name}");
            assert_eq!(file.line_stats.deletions, Some(0));
            assert_eq!(
                file.line_stats.additions,
                Some(u32::from(name == "new.txt"))
            );
        }
        let tracked = repo
            .commit_to_worktree_files(&from, false, &cancellation)
            .unwrap();
        assert_eq!(tracked.len(), 1);
        assert_eq!(tracked[0].path, Path::new("old.txt"));
        assert_eq!(tracked[0].kind, FileStatusKind::Deleted);
        assert_eq!(
            std::fs::read(&index).unwrap(),
            before,
            "comparison must not modify the index"
        );

        let saved_index = dir.join(".git/index.saved");
        std::fs::rename(&index, &saved_index).unwrap();
        let files = repo
            .commit_to_worktree_files(&from, true, &cancellation)
            .unwrap();
        assert_eq!(files.len(), 3);
        let retained: Vec<_> = files
            .iter()
            .filter(|file| file.path == Path::new("old.txt"))
            .collect();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].kind, FileStatusKind::Modified);
        assert!(
            !index.exists(),
            "comparison recreated the missing source index"
        );
        std::fs::rename(saved_index, &index).unwrap();

        assert!(
            intent_to_add_index(dir, &repo.repo(), &["absent.txt".into()], &cancellation).is_err(),
            "if preparing the comparison fails, return the Git error"
        );
        assert_eq!(
            std::fs::read(&index).unwrap(),
            before,
            "failed temporary index keeps the original index"
        );
    }

    /// Untracked files are counted from disk under the cap the tracked lanes
    /// use; past it they report no counts instead of being read whole.
    #[test]
    fn untracked_counts_stop_at_the_worktree_size_cap() {
        use crate::test_support::{git_success, init_test_repo, open_repo, write_file};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, "tracked.txt", "one\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        write_file(dir, "small.txt", "a\nb\n");
        let oversized = "line\n".repeat(super::super::log::COMMIT_STATS_MAX_BLOB_BYTES / 5 + 1);
        std::fs::write(dir.join("big.txt"), oversized).unwrap();

        let files = against_the_worktree_with_untracked(&open_repo(dir));
        let counts = |name: &str| {
            let file = files
                .iter()
                .find(|file| file.path == Path::new(name))
                .unwrap();
            (
                file.kind,
                file.line_stats.additions,
                file.line_stats.deletions,
            )
        };
        assert_eq!(
            counts("small.txt"),
            (FileStatusKind::Untracked, Some(2), Some(0))
        );
        assert_eq!(counts("big.txt"), (FileStatusKind::Untracked, None, None));
    }

    /// The comparison counts an untracked file the way the status list does:
    /// as git would store it, so a symlink is its link text, and a path the
    /// LFS filter would clean is unknown rather than cleaned.
    #[cfg(unix)]
    #[test]
    fn untracked_counts_match_the_status_list() {
        use crate::test_support::{git_success, init_test_repo, open_repo, write_file};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        write_file(dir, ".gitattributes", "*.bin filter=lfs\n");
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-m", "seed"]);
        git_success(dir, &["config", "filter.lfs.clean", "cat"]);
        write_file(dir, "three.txt", "a\nb\nc\n");
        std::os::unix::fs::symlink("three.txt", dir.join("link")).unwrap();
        write_file(dir, "new.bin", "payload\n");

        let repo = open_repo(dir);
        let status = repo
            .uncommitted_line_stats_impl(&CancellationToken::new())
            .unwrap()
            .unstaged;
        let files = against_the_worktree_with_untracked(&repo);
        for path in ["three.txt", "link", "new.bin"] {
            let file = files
                .iter()
                .find(|file| file.path == Path::new(path))
                .unwrap();
            let listed = status[Path::new(path)];
            assert_eq!(
                (file.kind, file.line_stats),
                (FileStatusKind::Untracked, listed),
                "{path}"
            );
        }
        assert_eq!(status[Path::new("link")].additions, Some(1), "link text");
        assert_eq!(status[Path::new("new.bin")].additions, None);
    }

    /// Past the edit cap the comparison drops the edits of its tracked files,
    /// not the counts of its untracked ones.
    #[test]
    fn untracked_counts_survive_the_edit_cap() {
        use crate::test_support::{git_success, init_test_repo, open_repo, write_file};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        init_test_repo(dir);
        let tracked = super::super::log::COMMIT_STATS_MAX_FILES + 1;
        for ix in 0..tracked {
            write_file(dir, &format!("tracked/{ix:03}.txt"), "base\n");
        }
        git_success(dir, &["add", "."]);
        git_success(dir, &["commit", "-q", "-m", "seed"]);
        for ix in 0..tracked {
            write_file(dir, &format!("tracked/{ix:03}.txt"), "edited\n");
        }
        write_file(dir, "new.txt", "a\nb\n");

        let files = against_the_worktree_with_untracked(&open_repo(dir));
        let file = |path: &str| {
            files
                .iter()
                .find(|file| file.path == Path::new(path))
                .unwrap()
        };

        let new = file("new.txt");
        assert_eq!(
            (new.line_stats.additions, new.line_stats.deletions),
            (Some(2), Some(0))
        );
        assert!(
            new.line_stats.edit.is_some(),
            "the edit comes with the counts"
        );
        let edited = file("tracked/000.txt");
        assert_eq!(edited.line_stats, (Some(1), Some(1)).into());
    }

    fn against_the_worktree_with_untracked(repo: &GixRepo) -> Vec<CommitFileChange> {
        let head = super::super::history::gix_head_id_or_none(&repo.repo())
            .unwrap()
            .unwrap();
        repo.compare_files_impl(
            &CommitId(head.to_string().into()),
            None,
            &ComparisonOptions::direct().with_untracked(true),
            &CancellationToken::new(),
        )
        .unwrap()
        .files
    }

    #[test]
    fn raw_records_keep_rename_sources_ids_and_modes() {
        let zero = "0".repeat(40);
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let output = format!(
            ":100644 100755 {a} {b} R087\0old/name.txt\0new/name.txt\0\
             :000000 160000 {zero} {b} A\0vendor/lib\0\
             :100644 100644 {a} {zero} M\0edited.txt\0"
        );
        let changes = parse_raw_changes(output.as_bytes(), &CancellationToken::new()).unwrap();
        assert_eq!(changes.len(), 3);

        let rename = &changes[0];
        assert_eq!(rename.path, PathBuf::from("new/name.txt"));
        assert_eq!(rename.old_path, Some(PathBuf::from("old/name.txt")));
        assert_eq!(rename.kind, FileStatusKind::Renamed);
        assert_eq!(rename.old_mode, Some(FileMode::Regular));
        assert_eq!(rename.new_mode, Some(FileMode::Executable));
        assert_eq!(rename.old_id.as_ref().map(AsRef::as_ref), Some(a.as_str()));

        let submodule = &changes[1];
        assert!(submodule.is_submodule);
        assert_eq!(submodule.old_id, None, "the zero id is no object");
        assert_eq!(submodule.old_mode, None);
        assert_eq!(submodule.new_mode, Some(FileMode::Gitlink));

        // A working-tree side has no object id yet.
        assert_eq!(changes[2].new_id, None);
        assert_eq!(changes[2].old_path, None);
    }
}
