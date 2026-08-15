//! Production command dispatch. `apxm-dev` does not compile this module.

use anyhow::Result;

use super::agent::{agent_install, agent_lint, agent_new, agent_sync, agent_verify};
use super::org::{org_install, org_lint, org_new};
use super::{AgentAction, OrgAction};

pub fn agent_command(action: AgentAction, json_output: bool) -> Result<()> {
    match action {
        AgentAction::New {
            id,
            path,
            display_name,
            template,
        } => agent_new(&id, path, display_name, &template, json_output),
        AgentAction::Sync { path } => agent_sync(&path, json_output),
        AgentAction::Lint { path, org } => agent_lint(&path, org, json_output),
        AgentAction::Install { path, force } => agent_install(&path, force, json_output),
        AgentAction::Verify { path } => agent_verify(&path, json_output),
    }
}

pub fn org_command(action: OrgAction, json_output: bool) -> Result<()> {
    match action {
        OrgAction::New {
            id,
            path,
            display_name,
        } => org_new(&id, path, display_name, json_output),
        OrgAction::Lint { path } => org_lint(&path, json_output),
        OrgAction::Install { path, force } => org_install(&path, force, json_output),
    }
}
