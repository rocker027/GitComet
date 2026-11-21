//! Keeps fetches and pulls from moving branches an active Worktree Finish run protects.
//!
//! Git writes every destination of a remote's configured `remote.<name>.fetch` refspecs on
//! any fetch or pull that contacts the remote, even one naming its own refspecs, so a
//! configuration such as `+refs/heads/main:refs/heads/release` would move a protected local
//! branch behind the run's back. Remote-tracking destinations (`refs/remotes/*`) never match.

use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitRepository, Result};

/// Whether writing fetch destination `dst` could move a protected local branch.
///
/// `run_active` says an active run guards this repository, which is enough for a wildcard
/// that can reach `refs/heads/*`; `protects` decides a single branch name. `HEAD` and `@`
/// are left to the caller, which knows whether the worktree's own HEAD is protected.
pub(super) fn fetch_destination_writes_protected_branch(
    dst: &str,
    run_active: bool,
    protects: impl Fn(&str) -> bool,
) -> bool {
    let trimmed = dst.trim();
    if trimmed.is_empty() {
        return false;
    }
    let mut normalized = String::with_capacity(trimmed.len());
    let mut prev_slash = false;
    for ch in trimmed.chars() {
        if ch == '/' {
            if !prev_slash {
                normalized.push('/');
            }
            prev_slash = true;
        } else {
            normalized.push(ch);
            prev_slash = false;
        }
    }
    if let Some(branch_pat) = normalized
        .strip_prefix("refs/heads/")
        .or_else(|| normalized.strip_prefix("heads/"))
    {
        if branch_pat.contains('*') {
            return run_active;
        }
        return protects(branch_pat);
    }
    if normalized.starts_with("refs/") {
        if let Some((prefix, _)) = normalized.split_once('*') {
            return "refs/heads/".starts_with(prefix) && run_active;
        }
        return false;
    }
    if normalized.contains('*') {
        return run_active;
    }
    protects(&normalized)
}

/// Why fetching from `remote` must not run while `protected` branches are guarded, or `None`
/// when its configured refspecs only write unprotected refs. An unreadable configuration
/// cannot be proven safe and is refused too.
pub(super) fn protected_fetch_refusal(
    repo: &dyn GitRepository,
    remote: &str,
    protected: &[String],
) -> Option<String> {
    if protected.is_empty() {
        return None;
    }
    let destinations = match repo.configured_fetch_destinations(remote) {
        Ok(destinations) => destinations,
        Err(error) => {
            return Some(format!(
                "Fetch from '{remote}' was refused while a Worktree Finish run is active: its fetch refspecs could not be read ({error})."
            ));
        }
    };
    let writes_protected = |dst: &str| {
        if dst == "HEAD" || dst == "@" {
            return true;
        }
        fetch_destination_writes_protected_branch(dst, true, |branch| {
            protected.iter().any(|name| name == branch)
        })
    };
    destinations
        .iter()
        .find(|dst| writes_protected(dst))
        .map(|dst| {
            format!(
                "Fetch from '{remote}' was refused: its configured fetch refspec writes '{dst}', a branch an active Worktree Finish run protects. Fetch another remote or wait until the run finishes."
            )
        })
}

/// Refuses a manual fetch or pull when any remote from `remotes` would write a protected
/// branch. `remotes` is only read when a run protects something, so an idle repository
/// pays nothing.
pub(super) fn refuse_protected_fetch(
    repo: &dyn GitRepository,
    protected: &[String],
    remotes: impl FnOnce() -> Result<Vec<String>>,
) -> Result<()> {
    if protected.is_empty() {
        return Ok(());
    }
    let remotes = remotes().map_err(|error| {
        Error::new(ErrorKind::Backend(format!(
            "Fetch was refused while a Worktree Finish run is active: the remotes it contacts could not be listed ({error})."
        )))
    })?;
    match remotes
        .iter()
        .find_map(|remote| protected_fetch_refusal(repo, remote, protected))
    {
        Some(reason) => Err(Error::new(ErrorKind::Backend(reason))),
        None => Ok(()),
    }
}
