//! Operator-driven cross-graph concept alignment (aegis-sosiaa).
//!
//! Sharing moves a graph; it does not move an opinion about what the graph's
//! concepts ARE. This module is the step that closes that gap — propose,
//! decide, record — with the record held as a SSSOM mapping set OUTSIDE both
//! source graphs, so an imported graph stays byte-recoverable against its own
//! share hash.
//!
//! Design: `docs/design/cross-graph-alignment.md`.

pub mod enumerate;
pub mod propose;
pub mod sssom;
pub mod verify;

#[cfg(test)]
mod tests;
