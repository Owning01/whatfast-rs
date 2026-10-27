//! The guide of an agent, as the screen installs it.
//!
//! The guide is a short block of rules —how to talk to the account, what an
//! agent may not do— that Whatfast writes into the file each harness reads by
//! itself: `CLAUDE.md`, `AGENTS.md`, the rules of Cursor. Installing it from the
//! screen is one click; the text and the per-harness paths live in
//! `whatfast_agents::guide`.
//!
//! Only the user writes it: it is configuration of the machine, not something
//! an agent changes about itself.

use std::path::Path;

use whatfast_protocol::{
    GuideInstallRequest, GuideInstallResponse, GuideRemoveRequest, GuideRemoveResponse,
    GuideStatusRequest, GuideStatusResponse, RpcResponse, rpc_response,
};

use crate::guard::{Denied, Who};

/// Writes or refreshes the block in the rules file of the harness.
pub fn install(
    who: &Who,
    request: &GuideInstallRequest,
    exe: &Path,
) -> Result<RpcResponse, Denied> {
    user_only(who)?;
    let spec = spec_of(&request.agent_id)?;
    let report = whatfast_agents::guide::install(spec, exe).map_err(io_denied)?;
    let installed = whatfast_agents::guide::installed(spec);
    Ok(answer(rpc_response::Result::GuideInstall(
        GuideInstallResponse {
            installed,
            target: report.target,
            backup: report
                .backup
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        },
    )))
}

/// Takes the block out of the rules file.
pub fn remove(who: &Who, request: &GuideRemoveRequest) -> Result<RpcResponse, Denied> {
    user_only(who)?;
    let spec = spec_of(&request.agent_id)?;
    let removed = whatfast_agents::guide::remove(spec).map_err(io_denied)?;
    Ok(answer(rpc_response::Result::GuideRemove(
        GuideRemoveResponse { removed },
    )))
}

/// Whether the block is in the rules file, and where that file is.
pub fn status(who: &Who, request: &GuideStatusRequest) -> Result<RpcResponse, Denied> {
    user_only(who)?;
    let spec = spec_of(&request.agent_id)?;
    let target = whatfast_agents::guide::rules_file(spec)
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    Ok(answer(rpc_response::Result::GuideStatus(
        GuideStatusResponse {
            installed: whatfast_agents::guide::installed(spec),
            target,
        },
    )))
}

/// The harness a request names.
fn spec_of(agent_id: &str) -> Result<&'static whatfast_agents::ClientSpec, Denied> {
    let id = agent_id.trim();
    whatfast_agents::by_id(id).ok_or_else(|| Denied {
        code: "invalid_argument",
        message: format!("`{id}` is not a harness this build knows"),
    })
}

/// The guide is the user's: an agent does not write the rules of the machine.
fn user_only(who: &Who) -> Result<(), Denied> {
    match who {
        Who::User => Ok(()),
        Who::Agent { id, .. } => Err(Denied {
            code: "forbidden",
            message: format!("{id} cannot write the guide of the profile"),
        }),
        Who::Denied(reason) => Err(Denied {
            code: "unauthorized",
            message: reason.clone(),
        }),
    }
}

/// The failure of the file, as the answer of a request.
fn io_denied(error: std::io::Error) -> Denied {
    Denied {
        code: "internal",
        message: format!("the rules file: {error}"),
    }
}

/// The answer of one of the three requests.
fn answer(result: rpc_response::Result) -> RpcResponse {
    RpcResponse {
        result: Some(result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_harness_is_refused() {
        let denied = spec_of("no-existe").expect_err("unknown");
        assert_eq!(denied.code, "invalid_argument");
    }

    #[test]
    fn a_known_harness_is_found_by_id() {
        let spec = spec_of("claude-code").expect("claude-code");
        assert_eq!(spec.id, "claude-code");
    }

    #[test]
    fn the_guide_is_the_users() {
        let who = Who::Agent {
            id: "codex".into(),
            grant: whatfast_auto::Grant::default(),
        };
        let request = GuideStatusRequest {
            agent_id: "codex".into(),
        };
        let denied = status(&who, &request).expect_err("forbidden");
        assert_eq!(denied.code, "forbidden");
    }
}
