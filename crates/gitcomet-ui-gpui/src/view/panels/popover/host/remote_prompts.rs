//! Remote prompts: add a remote, edit its URL, and push with or set an
//! upstream.

use super::*;

pub(in crate::view::panels::popover) fn upstream_prompt_submission(
    kind: &PopoverKind,
    remote: String,
    remote_branch: String,
) -> Option<Msg> {
    let PopoverKind::PushSetUpstreamPrompt { repo_id, mode, .. } = kind else {
        return None;
    };
    Some(match mode {
        PushUpstreamMode::ConfigureOnly(local_branch) => Msg::SetUpstreamBranch {
            repo_id: *repo_id,
            branch: local_branch.clone(),
            upstream: Upstream {
                remote,
                branch: remote_branch,
            },
        },
        PushUpstreamMode::SelectedBranch(local_branch) => Msg::PushBranch {
            repo_id: *repo_id,
            local_branch: local_branch.clone(),
            upstream: Upstream {
                remote,
                branch: remote_branch,
            },
            set_upstream: true,
        },
        PushUpstreamMode::CurrentBranch => Msg::PushSetUpstream {
            repo_id: *repo_id,
            remote,
            branch: remote_branch,
        },
    })
}

impl PopoverHost {
    pub(in crate::view::panels::popover) fn can_submit_remote_add(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.remote_name_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
            && self
                .remote_url_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn submit_remote_add(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Remote(RemotePopoverKind::AddPrompt),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_remote_add(cx) {
            return;
        }
        let name = self
            .remote_name_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let url = self
            .remote_url_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        self.store.dispatch(Msg::AddRemote { repo_id, name, url });
        self.close_popover(cx);
    }

    pub(in crate::view::panels::popover) fn can_submit_remote_edit_url(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        self.remote_url_edit_input
            .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn submit_remote_edit_url(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::Repo {
            repo_id,
            kind: RepoPopoverKind::Remote(RemotePopoverKind::EditUrlPrompt { name, kind }),
        }) = self.popover.clone()
        else {
            return;
        };
        if !self.can_submit_remote_edit_url(cx) {
            return;
        }
        let url = self
            .remote_url_edit_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        self.store.dispatch(Msg::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        });
        self.close_popover(cx);
    }

    pub(in crate::view::panels::popover) fn can_submit_push_set_upstream(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(PopoverKind::PushSetUpstreamPrompt { repo_id, mode, .. }) = self.popover.as_ref()
        else {
            return false;
        };
        let local_branch_available = mode.local_branch().is_none_or(|branch| {
            let Some(repo) = self.state.repos.iter().find(|repo| repo.id == *repo_id) else {
                return false;
            };
            let Some(candidate) = repo
                .branches
                .ready()
                .and_then(|branches| branches.iter().find(|candidate| candidate.name == branch))
            else {
                return false;
            };
            match mode {
                PushUpstreamMode::SelectedBranch(_) => {
                    candidate.upstream.is_none() && !repo.local_branch_push_blocked_by_annex(branch)
                }
                _ => repo.head_branch.ready().is_some_and(|head| head == branch),
            }
        });
        local_branch_available
            && self.selected_push_upstream_remote().is_some()
            && self
                .push_upstream_branch_input
                .read_with(cx, |i, _| !i.text().trim().is_empty())
    }

    pub(in crate::view::panels::popover) fn selected_push_upstream_remote(&self) -> Option<String> {
        let PopoverKind::PushSetUpstreamPrompt {
            repo_id,
            remote,
            mode,
        } = self.popover.as_ref()?
        else {
            return None;
        };
        let repo = self.state.repos.iter().find(|repo| repo.id == *repo_id)?;
        let selected = push_set_upstream_prompt::selected_remote(repo, remote)?;
        if matches!(mode, PushUpstreamMode::SelectedBranch(_)) && selected != *remote {
            return None;
        }
        Some(selected)
    }

    pub(in crate::view::panels::popover) fn select_push_upstream_remote(
        &mut self,
        selected: String,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(PopoverKind::PushSetUpstreamPrompt {
            repo_id, remote, ..
        }) = self.popover.as_mut()
        else {
            return;
        };
        let is_configured = self
            .state
            .repos
            .iter()
            .find(|repo| repo.id == *repo_id)
            .map(push_set_upstream_prompt::remote_names)
            .is_some_and(|names| names.contains(&selected));
        if is_configured {
            *remote = selected;
        }
        self.sync_tag_push_previews(cx);
        self.push_upstream_remote_menu_open = false;
        self.push_upstream_remote_selected_index = None;
        cx.notify();
    }

    pub(in crate::view::panels::popover) fn submit_push_set_upstream(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(kind @ PopoverKind::PushSetUpstreamPrompt { .. }) = self.popover.clone() else {
            return;
        };
        if !self.can_submit_push_set_upstream(cx) {
            return;
        }
        let Some(remote) = self.selected_push_upstream_remote() else {
            return;
        };
        let branch = self
            .push_upstream_branch_input
            .read_with(cx, |i, _| i.text().trim().to_string());
        let message = if let (
            Some(mode),
            PopoverKind::PushSetUpstreamPrompt {
                repo_id,
                mode: PushUpstreamMode::CurrentBranch,
                ..
            },
        ) = (self.push_upstream_tag_mode, &kind)
        {
            let Some(repo) = self.state.repos.iter().find(|repo| repo.id == *repo_id) else {
                return;
            };
            let Some(mut request) = tag_push::request(repo, mode, &self.state.large_file_settings)
            else {
                return;
            };
            request.remote = remote;
            request.branch = branch;
            request.set_upstream = true;
            Msg::PushWithTags {
                repo_id: *repo_id,
                request,
            }
        } else {
            let Some(message) = upstream_prompt_submission(&kind, remote, branch) else {
                return;
            };
            message
        };
        self.store.dispatch(message);
        self.close_popover(cx);
    }
}
