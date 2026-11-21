//! Undo and redo of GitComet's own ref-moving operations, for one run of the
//! app: commits, creating and deleting branches, and soft resets.
//!
//! Each entry records the refs an operation moved, with their values before
//! and after. Undo writes the before values back in one `git update-ref`
//! transaction that expects the after values, so a ref moved since by
//! anything else makes the whole transaction fail instead of overwriting it.
//!
//! Only the last operations count: one the journal does not record that
//! moves a branch or HEAD (a checkout, merge, rebase, hard reset...) clears
//! it, so Undo never reaches past it.
use super::{Loadable, RepoState};
use gitcomet_core::domain::CommitId;
use gitcomet_core::services::RefUpdate;

/// Entries kept per repository; the oldest are dropped first.
pub const GIT_UNDO_LIMIT: usize = 50;

/// One ref an operation moved. `None` means the ref did not exist.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRefChange {
    pub reference: String,
    pub before: Option<CommitId>,
    pub after: Option<CommitId>,
}

impl GitRefChange {
    /// The local branch the ref names, if it is one.
    pub fn branch(&self) -> Option<&str> {
        self.reference.strip_prefix("refs/heads/")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitUndoEntry {
    /// What the operation was, for "Undo <label>".
    pub label: String,
    pub changes: Vec<GitRefChange>,
    /// The branch it moved must be checked out here to undo or redo it: a
    /// commit or soft reset moves the checked-out branch, and undoing it
    /// anywhere else would drop commits from every branch.
    pub requires_head: bool,
}

/// What the worker confirms against the repository itself right before the
/// transaction, since the state an undo was planned from can lag behind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitUndoCheck {
    pub branch: String,
    /// The branch must be the one checked out here.
    pub must_be_head: bool,
    /// The transaction deletes the branch, so it must not be checked out.
    pub deletes: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitUndoDirection {
    Undo,
    Redo,
}

impl GitUndoDirection {
    /// The repository action that runs this direction's transaction.
    pub fn action_kind(direction: Self) -> crate::msg::RepoActionKind {
        match direction {
            Self::Undo => crate::msg::RepoActionKind::UndoGitOperation,
            Self::Redo => crate::msg::RepoActionKind::RedoGitOperation,
        }
    }
}

/// An operation whose completion message does not carry the refs it moved:
/// its entry is prepared when it starts and recorded when it succeeds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingGitUndoKind {
    CreateBranch,
    DeleteBranch,
    ForceDeleteBranch,
    SoftReset,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GitUndoJournal {
    /// Entries with the id they were recorded under, oldest first.
    undo: Vec<(u64, GitUndoEntry)>,
    redo: Vec<(u64, GitUndoEntry)>,
    pending: Option<(PendingGitUndoKind, GitUndoEntry)>,
    /// The running transaction and the entry it applies.
    in_flight: Option<(GitUndoDirection, u64)>,
    next_id: u64,
}

impl GitUndoJournal {
    pub fn next_undo(&self) -> Option<&GitUndoEntry> {
        self.undo.last().map(|(_, entry)| entry)
    }

    pub fn next_redo(&self) -> Option<&GitUndoEntry> {
        self.redo.last().map(|(_, entry)| entry)
    }

    pub fn next(&self, direction: GitUndoDirection) -> Option<&GitUndoEntry> {
        match direction {
            GitUndoDirection::Undo => self.next_undo(),
            GitUndoDirection::Redo => self.next_redo(),
        }
    }

    pub fn in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Changes whenever what Undo and Redo would do changes, for views that
    /// repaint on it.
    pub fn summary(&self) -> (usize, usize, bool) {
        (self.undo.len(), self.redo.len(), self.in_flight.is_some())
    }

    /// Records `entry` as a finished operation, for tests outside this crate.
    #[cfg(any(test, feature = "test-support"))]
    pub fn record_for_test(&mut self, entry: GitUndoEntry) {
        self.record(entry);
    }

    /// A new operation succeeded: it can be undone, and what was undone
    /// before it can no longer be redone.
    pub(crate) fn record(&mut self, entry: GitUndoEntry) {
        if entry.changes.is_empty() {
            return;
        }
        self.next_id += 1;
        self.undo.push((self.next_id, entry));
        if self.undo.len() > GIT_UNDO_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// An operation the journal does not record moved a branch or HEAD:
    /// nothing before it can be undone or redone any more.
    pub(crate) fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.pending = None;
    }

    /// The worker read `reference` from the repository as the pending
    /// operation of `kind` started: that is the value it moves from. A ref
    /// other than the planned one, or a missing one, leaves nothing to
    /// record.
    pub(crate) fn observe_pending(
        &mut self,
        kind: PendingGitUndoKind,
        reference: &str,
        value: Option<CommitId>,
    ) {
        let Some((pending_kind, entry)) = self.pending.as_mut() else {
            return;
        };
        if *pending_kind != kind {
            return;
        }
        match (entry.changes.as_mut_slice(), value) {
            ([change], Some(value)) if change.reference == reference => {
                change.before = Some(value);
            }
            _ => self.pending = None,
        }
    }

    /// Prepares the entry of an operation that is starting. Only one can be
    /// pending: a second, or one started while other local work runs, could
    /// be confused with the first's completion, so neither is recorded.
    pub(crate) fn begin_pending(
        &mut self,
        kind: PendingGitUndoKind,
        entry: Option<GitUndoEntry>,
        other_work_running: bool,
    ) {
        if self.pending.is_some() || other_work_running {
            self.pending = None;
            return;
        }
        self.pending = entry.map(|entry| (kind, entry));
    }

    /// An operation of `kind` finished; its prepared entry is recorded when
    /// it succeeded. Completions of other kinds leave the pending entry.
    pub(crate) fn finish_pending(&mut self, kind: PendingGitUndoKind, succeeded: bool) {
        if self
            .pending
            .as_ref()
            .is_none_or(|(pending_kind, _)| *pending_kind != kind)
        {
            return;
        }
        if let Some((_, entry)) = self.pending.take()
            && succeeded
        {
            self.record(entry);
        }
    }

    /// Starts undoing or redoing the next entry, returning the ref
    /// transaction to run; `None` when there is nothing to do or one is
    /// already running.
    pub(crate) fn begin(
        &mut self,
        direction: GitUndoDirection,
    ) -> Option<(Vec<RefUpdate>, Vec<GitUndoCheck>)> {
        if self.in_flight.is_some() {
            return None;
        }
        let (id, entry) = match direction {
            GitUndoDirection::Undo => self.undo.last()?,
            GitUndoDirection::Redo => self.redo.last()?,
        };
        let planned = (ref_updates(entry, direction), checks(entry, direction));
        self.in_flight = Some((direction, *id));
        Some(planned)
    }

    /// The transaction finished. On success its entry moves to the other
    /// stack; on failure it stays, so the user can see why and try again.
    /// The entry is found by id: an operation recorded meanwhile stays put.
    pub(crate) fn finish(&mut self, direction: GitUndoDirection, succeeded: bool) {
        let Some((running, id)) = self.in_flight else {
            return;
        };
        if running != direction {
            return;
        }
        self.in_flight = None;
        if !succeeded {
            return;
        }
        let (from, to) = match direction {
            GitUndoDirection::Undo => (&mut self.undo, &mut self.redo),
            GitUndoDirection::Redo => (&mut self.redo, &mut self.undo),
        };
        if let Some(index) = from.iter().position(|(entry_id, _)| *entry_id == id) {
            to.push(from.remove(index));
        }
    }
}

/// The ref transaction that undoes (or redoes) `entry`: each ref must still
/// hold the value the operation left (or the one undo restored), and is set
/// back. A missing value is all zeroes, which `git update-ref` reads as
/// "must not exist" when expected and as "delete" when written.
pub fn ref_updates(entry: &GitUndoEntry, direction: GitUndoDirection) -> Vec<RefUpdate> {
    entry
        .changes
        .iter()
        .map(|change| {
            let (from, to) = match direction {
                GitUndoDirection::Undo => (&change.after, &change.before),
                GitUndoDirection::Redo => (&change.before, &change.after),
            };
            let zero = || {
                let len = change
                    .before
                    .as_ref()
                    .or(change.after.as_ref())
                    .map_or(40, |id| id.as_ref().len());
                CommitId("0".repeat(len).into())
            };
            RefUpdate {
                reference: change.reference.clone(),
                expected: from.clone(),
                new: to.clone().unwrap_or_else(zero),
            }
        })
        .collect()
}

/// What the worker must confirm before running `entry` in `direction`.
pub fn checks(entry: &GitUndoEntry, direction: GitUndoDirection) -> Vec<GitUndoCheck> {
    entry
        .changes
        .iter()
        .filter_map(|change| {
            let branch = change.branch()?;
            let deletes = match direction {
                GitUndoDirection::Undo => change.before.is_none(),
                GitUndoDirection::Redo => change.after.is_none(),
            };
            Some(GitUndoCheck {
                branch: branch.to_string(),
                must_be_head: entry.requires_head,
                deletes,
            })
        })
        .collect()
}

/// Why Undo (or Redo) cannot run now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitUndoBlock {
    Nothing(GitUndoDirection),
    /// Other Git work is running.
    Busy,
    /// The checked-out branch or the worktrees are not loaded yet.
    Loading,
    /// The entry moved a branch that is no longer checked out here.
    NotCheckedOut(String),
    CheckedOutElsewhere(String),
    /// It would delete the checked-out branch.
    DeletesCheckedOut(String),
}

impl GitUndoBlock {
    /// A short reason for a disabled control.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Nothing(GitUndoDirection::Undo) => "Nothing to undo",
            Self::Nothing(GitUndoDirection::Redo) => "Nothing to redo",
            Self::Busy => "Wait for the running Git operation to finish",
            Self::Loading => "Wait for the repository to finish loading",
            Self::NotCheckedOut(_) => "Check out the branch it changed first",
            Self::CheckedOutElsewhere(_) => "Its branch is checked out in another worktree",
            Self::DeletesCheckedOut(_) => "It would delete the checked-out branch",
        }
    }

    /// The full sentence for `entry`, for the error the request shows.
    pub fn message(&self, entry: &GitUndoEntry, direction: GitUndoDirection) -> String {
        let verb = match direction {
            GitUndoDirection::Undo => "undo",
            GitUndoDirection::Redo => "redo",
        };
        let label = &entry.label;
        match self {
            Self::NotCheckedOut(branch) => {
                format!("Cannot {verb} \"{label}\": check out {branch} first")
            }
            Self::CheckedOutElsewhere(branch) => {
                format!("Cannot {verb} \"{label}\": {branch} is checked out in another worktree")
            }
            Self::DeletesCheckedOut(branch) => {
                format!("Cannot {verb} \"{label}\": {branch} is checked out")
            }
            other => format!("Cannot {verb} \"{label}\": {}", other.reason()),
        }
    }
}

impl RepoState {
    /// Whether Git work runs that an undo must not overlap, or that must not
    /// be mistaken for the completion of a recorded operation.
    pub fn git_undo_busy(&self) -> bool {
        self.git_undo.in_flight()
            || self.local_actions_in_flight > 0
            || self.sequencer_actions_in_flight > 0
            || self.commit_in_flight > 0
            || self.pull_in_flight > 0
            || self.worktree_pull_in_flight > 0
            || self.push_in_flight > 0
            || self.history_rewrite_busy()
    }

    /// Why Undo (or Redo) cannot run now, judged from the loaded state; the
    /// worker checks the repository again before it writes anything.
    pub fn git_undo_block(&self, direction: GitUndoDirection) -> Option<GitUndoBlock> {
        let Some(entry) = self.git_undo.next(direction) else {
            return Some(GitUndoBlock::Nothing(direction));
        };
        if self.git_undo_busy() {
            return Some(GitUndoBlock::Busy);
        }
        let (Loadable::Ready(head), Loadable::Ready(worktrees)) =
            (&self.head_branch, &self.worktrees)
        else {
            return Some(GitUndoBlock::Loading);
        };
        for check in checks(entry, direction) {
            let branch = check.branch;
            if check.must_be_head && *head != branch {
                return Some(GitUndoBlock::NotCheckedOut(branch));
            }
            if check.deletes && *head == branch {
                return Some(GitUndoBlock::DeletesCheckedOut(branch));
            }
            let elsewhere = worktrees.iter().any(|worktree| {
                !worktree.bare
                    && worktree.path != self.spec.workdir
                    && worktree.branch.as_deref() == Some(branch.as_str())
            });
            if elsewhere {
                return Some(GitUndoBlock::CheckedOutElsewhere(branch));
            }
        }
        None
    }
}

/// Whether `value` is a full commit id (SHA-1 or SHA-256 hex), the only
/// form a recorded ref value may take.
pub(crate) fn is_full_commit_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(c: char) -> CommitId {
        CommitId(c.to_string().repeat(40).into())
    }

    fn entry(before: Option<CommitId>, after: Option<CommitId>) -> GitUndoEntry {
        GitUndoEntry {
            label: "Commit".into(),
            changes: vec![GitRefChange {
                reference: "refs/heads/main".into(),
                before,
                after,
            }],
            requires_head: false,
        }
    }

    #[test]
    fn undo_moves_a_ref_back_expecting_where_the_operation_left_it() {
        let updates = ref_updates(&entry(Some(id('a')), Some(id('b'))), GitUndoDirection::Undo);
        assert_eq!(
            updates,
            vec![RefUpdate {
                reference: "refs/heads/main".into(),
                expected: Some(id('b')),
                new: id('a'),
            }]
        );
        let redo = ref_updates(&entry(Some(id('a')), Some(id('b'))), GitUndoDirection::Redo);
        assert_eq!(redo[0].expected, Some(id('a')));
        assert_eq!(redo[0].new, id('b'));
    }

    #[test]
    fn created_and_deleted_branches_use_zero_for_absent() {
        // Undoing a created branch deletes it, expecting its tip.
        let created = ref_updates(&entry(None, Some(id('b'))), GitUndoDirection::Undo);
        assert_eq!(created[0].expected, Some(id('b')));
        assert_eq!(created[0].new, id('0'));
        // Undoing a deleted branch recreates it, requiring it to be absent.
        let deleted = ref_updates(&entry(Some(id('a')), None), GitUndoDirection::Undo);
        assert_eq!(deleted[0].expected, None);
        assert_eq!(deleted[0].new, id('a'));
    }

    #[test]
    fn entries_move_between_stacks_only_on_success() {
        let mut journal = GitUndoJournal::default();
        journal.record(entry(Some(id('a')), Some(id('b'))));
        assert!(journal.begin(GitUndoDirection::Undo).is_some());
        assert!(journal.in_flight());
        // A second request while one runs is refused.
        assert!(journal.begin(GitUndoDirection::Undo).is_none());
        journal.finish(GitUndoDirection::Undo, false);
        assert!(
            journal.next_undo().is_some(),
            "a failed undo keeps the entry"
        );
        assert!(journal.begin(GitUndoDirection::Undo).is_some());
        journal.finish(GitUndoDirection::Undo, true);
        assert!(journal.next_undo().is_none());
        assert!(journal.next_redo().is_some());
        // A new operation clears what could be redone.
        journal.record(entry(Some(id('c')), Some(id('d'))));
        assert!(journal.next_redo().is_none());
    }

    #[test]
    fn pending_entries_are_recorded_only_for_their_own_success() {
        let mut journal = GitUndoJournal::default();
        journal.begin_pending(
            PendingGitUndoKind::DeleteBranch,
            Some(entry(Some(id('a')), None)),
            false,
        );
        // Another kind's completion leaves it pending.
        journal.finish_pending(PendingGitUndoKind::CreateBranch, true);
        assert!(journal.next_undo().is_none());
        journal.finish_pending(PendingGitUndoKind::DeleteBranch, true);
        assert!(journal.next_undo().is_some());

        let mut failed = GitUndoJournal::default();
        failed.begin_pending(
            PendingGitUndoKind::DeleteBranch,
            Some(entry(Some(id('a')), None)),
            false,
        );
        failed.finish_pending(PendingGitUndoKind::DeleteBranch, false);
        assert!(failed.next_undo().is_none());
    }

    #[test]
    fn overlapping_operations_are_not_recorded() {
        let mut journal = GitUndoJournal::default();
        journal.begin_pending(
            PendingGitUndoKind::DeleteBranch,
            Some(entry(Some(id('a')), None)),
            false,
        );
        journal.begin_pending(
            PendingGitUndoKind::DeleteBranch,
            Some(entry(Some(id('b')), None)),
            false,
        );
        journal.finish_pending(PendingGitUndoKind::DeleteBranch, true);
        assert!(journal.next_undo().is_none());

        let mut busy = GitUndoJournal::default();
        busy.begin_pending(
            PendingGitUndoKind::SoftReset,
            Some(entry(Some(id('a')), Some(id('b')))),
            true,
        );
        busy.finish_pending(PendingGitUndoKind::SoftReset, true);
        assert!(busy.next_undo().is_none());
    }

    #[test]
    fn full_commit_ids_only() {
        assert!(is_full_commit_id(&"a".repeat(40)));
        assert!(is_full_commit_id(&"0".repeat(64)));
        assert!(!is_full_commit_id("HEAD~1"));
        assert!(!is_full_commit_id("abc123"));
    }
}
