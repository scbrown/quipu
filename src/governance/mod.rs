//! Governance enforcement — binding the declarative policy vocabulary to the
//! write path (the loom's write-time gate).
//!
//! The declarative model (`aegis:Policy` with `boundary`/`claim`/`effect`) lives
//! in `shapes/governance.ttl` and is evaluated on demand by
//! [`crate::tool_policy_check`]. This module is the *enforcement* half: it binds
//! `boundary:"action"` policies to `Store::transact` so an edit that leaves a
//! governed entity non-compliant is rejected before commit.
//!
//! See `docs/design/policy-edit-hooks.md`.

pub mod audit;
pub mod audit_spec;
pub mod authority;
pub mod guard;
pub mod inventory;
pub mod placement;
pub mod router;
pub mod verdict_facts;

pub use guard::{PolicyRegistry, is_governance_write};
pub use placement::validate_write as validate_placement;
