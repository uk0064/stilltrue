//! Stage 2: decide whether each claim is still true.
//!
//! Invariant: `resolve` never invokes git. Git is touched only in `gate`, so a
//! repository with no broken claims spawns no git processes at all.

pub mod command;
pub mod lang;
pub mod symbol;
pub mod universal;

use crate::claim::{Claim, ClaimKind};
use crate::repo::Repo;
use crate::resolution::{Resolution, SkipReason};

pub fn resolve(claim: &Claim, repo: &Repo) -> Resolution {
    match &claim.kind {
        // Never reaches here: a suppression finding is built in the pipeline, not
        // resolved from a claim about the repository.
        ClaimKind::Suppression => Resolution::Skip(SkipReason::NotAClaim),
        ClaimKind::Path => universal::path(claim, repo),
        ClaimKind::Command { runner, args } => command::resolve(claim, repo, runner, args),
        ClaimKind::EnvVar => universal::env_var(claim, repo),
        ClaimKind::Version { tool, version } => universal::version(repo, tool, version),
        ClaimKind::Link { target, anchor } => {
            universal::link(claim, repo, target, anchor.as_deref())
        }
        ClaimKind::Symbol => symbol::resolve(claim, repo),
    }
}
