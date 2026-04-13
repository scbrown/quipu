//! Schema evolution proposals — agent-driven ontology change workflow.
//!
//! When an agent writes knowledge that fails SHACL validation because the
//! schema is too tight or missing a class/property, it can submit a structured
//! proposal for the change. Proposals persist in the `proposals` table and
//! follow a pending → accepted/rejected lifecycle.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::store::Store;

/// The kind of schema change being proposed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProposalKind {
    Shape,
    Ontology,
    Class,
    Property,
}

impl ProposalKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Shape => "shape",
            Self::Ontology => "ontology",
            Self::Class => "class",
            Self::Property => "property",
        }
    }

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "shape" => Ok(Self::Shape),
            "ontology" => Ok(Self::Ontology),
            "class" => Ok(Self::Class),
            "property" => Ok(Self::Property),
            other => Err(Error::InvalidValue(format!(
                "unknown proposal kind: {other}"
            ))),
        }
    }

    /// Parse from a JSON/MCP string value.
    pub fn from_json(s: &str) -> Result<Self> {
        Self::from_str(s)
    }
}

/// The lifecycle status of a proposal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Pending,
    Accepted,
    Rejected,
}

impl ProposalStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            other => Err(Error::InvalidValue(format!(
                "unknown proposal status: {other}"
            ))),
        }
    }

    /// Parse from a JSON/MCP string value.
    pub fn from_json(s: &str) -> Result<Self> {
        Self::from_str(s)
    }
}

/// A schema evolution proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: i64,
    pub kind: ProposalKind,
    pub target: String,
    pub diff: String,
    pub rationale: Option<String>,
    pub proposer: String,
    pub trigger_ref: Option<String>,
    pub status: ProposalStatus,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub decision_note: Option<String>,
    pub created_at: String,
}

/// Input for creating a new proposal.
pub struct NewProposal<'a> {
    pub kind: &'a ProposalKind,
    pub target: &'a str,
    pub diff: &'a str,
    pub rationale: Option<&'a str>,
    pub proposer: &'a str,
    pub trigger_ref: Option<&'a str>,
    pub created_at: &'a str,
}

impl Store {
    /// Insert a new schema change proposal. Returns the proposal id.
    pub fn insert_proposal(&self, input: &NewProposal<'_>) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO proposals (kind, target, diff, rationale, proposer, trigger_ref, status, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
                input.kind.as_str(),
                input.target,
                input.diff,
                input.rationale,
                input.proposer,
                input.trigger_ref,
                input.created_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get a proposal by id.
    pub fn get_proposal(&self, id: i64) -> Result<Option<Proposal>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, target, diff, rationale, proposer, trigger_ref, \
                    status, decided_by, decided_at, decision_note, created_at \
             FROM proposals WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_proposal(row)?)),
            None => Ok(None),
        }
    }

    /// List proposals, optionally filtered by status.
    pub fn list_proposals(&self, status: Option<&ProposalStatus>) -> Result<Vec<Proposal>> {
        let mut proposals = Vec::new();
        match status {
            Some(s) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, kind, target, diff, rationale, proposer, trigger_ref, \
                            status, decided_by, decided_at, decision_note, created_at \
                     FROM proposals WHERE status = ?1 ORDER BY created_at DESC",
                )?;
                let mut rows = stmt.query(params![s.as_str()])?;
                while let Some(row) = rows.next()? {
                    proposals.push(row_to_proposal(row)?);
                }
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, kind, target, diff, rationale, proposer, trigger_ref, \
                            status, decided_by, decided_at, decision_note, created_at \
                     FROM proposals ORDER BY created_at DESC",
                )?;
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    proposals.push(row_to_proposal(row)?);
                }
            }
        }
        Ok(proposals)
    }

    /// Accept a proposal. For shape proposals, validates the Turtle diff then
    /// writes it to the `shapes` table. Returns the updated proposal.
    #[cfg(feature = "shacl")]
    pub fn accept_proposal(
        &self,
        id: i64,
        decided_by: &str,
        decided_at: &str,
        note: Option<&str>,
    ) -> Result<Proposal> {
        let proposal = self
            .get_proposal(id)?
            .ok_or_else(|| Error::InvalidValue(format!("proposal {id} not found")))?;

        if proposal.status != ProposalStatus::Pending {
            return Err(Error::InvalidValue(format!(
                "proposal {id} is already {}",
                proposal.status.as_str()
            )));
        }

        // For shape proposals, validate the Turtle before accepting.
        if proposal.kind == ProposalKind::Shape {
            // First, verify the diff is syntactically valid Turtle.
            validate_turtle_syntax(&proposal.diff)?;

            // Then verify it parses as valid SHACL shapes.
            crate::shacl::Validator::from_turtle(&proposal.diff).map_err(|e| {
                Error::InvalidValue(format!(
                    "proposed shape Turtle is invalid, proposal remains pending: {e}"
                ))
            })?;

            // Write the shape to the shapes table.
            self.load_shapes(&proposal.target, &proposal.diff, decided_at)?;
        }

        self.conn.execute(
            "UPDATE proposals SET status = 'accepted', decided_by = ?1, decided_at = ?2, decision_note = ?3 \
             WHERE id = ?4",
            params![decided_by, decided_at, note, id],
        )?;

        self.get_proposal(id)?
            .ok_or_else(|| Error::InvalidValue(format!("proposal {id} vanished after update")))
    }

    /// Accept a proposal (non-SHACL build — no Turtle validation).
    #[cfg(not(feature = "shacl"))]
    pub fn accept_proposal(
        &self,
        id: i64,
        decided_by: &str,
        decided_at: &str,
        note: Option<&str>,
    ) -> Result<Proposal> {
        let proposal = self
            .get_proposal(id)?
            .ok_or_else(|| Error::InvalidValue(format!("proposal {id} not found")))?;

        if proposal.status != ProposalStatus::Pending {
            return Err(Error::InvalidValue(format!(
                "proposal {id} is already {}",
                proposal.status.as_str()
            )));
        }

        self.conn.execute(
            "UPDATE proposals SET status = 'accepted', decided_by = ?1, decided_at = ?2, decision_note = ?3 \
             WHERE id = ?4",
            params![decided_by, decided_at, note, id],
        )?;

        self.get_proposal(id)?
            .ok_or_else(|| Error::InvalidValue(format!("proposal {id} vanished after update")))
    }

    /// Reject a proposal with a reason.
    pub fn reject_proposal(
        &self,
        id: i64,
        decided_by: &str,
        decided_at: &str,
        note: &str,
    ) -> Result<Proposal> {
        let proposal = self
            .get_proposal(id)?
            .ok_or_else(|| Error::InvalidValue(format!("proposal {id} not found")))?;

        if proposal.status != ProposalStatus::Pending {
            return Err(Error::InvalidValue(format!(
                "proposal {id} is already {}",
                proposal.status.as_str()
            )));
        }

        self.conn.execute(
            "UPDATE proposals SET status = 'rejected', decided_by = ?1, decided_at = ?2, decision_note = ?3 \
             WHERE id = ?4",
            params![decided_by, decided_at, note, id],
        )?;

        self.get_proposal(id)?
            .ok_or_else(|| Error::InvalidValue(format!("proposal {id} vanished after update")))
    }
}

#[cfg(feature = "shacl")]
fn validate_turtle_syntax(turtle: &str) -> Result<()> {
    let parser = oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::Turtle);
    for quad_result in parser.for_reader(turtle.as_bytes()) {
        quad_result.map_err(|e| {
            Error::InvalidValue(format!(
                "proposed shape Turtle is invalid, proposal remains pending: {e}"
            ))
        })?;
    }
    Ok(())
}

fn row_to_proposal(row: &rusqlite::Row<'_>) -> Result<Proposal> {
    let kind_str: String = row.get(1)?;
    let status_str: String = row.get(7)?;
    Ok(Proposal {
        id: row.get(0)?,
        kind: ProposalKind::from_str(&kind_str)?,
        target: row.get(2)?,
        diff: row.get(3)?,
        rationale: row.get(4)?,
        proposer: row.get(5)?,
        trigger_ref: row.get(6)?,
        status: ProposalStatus::from_str(&status_str)?,
        decided_by: row.get(8)?,
        decided_at: row.get(9)?,
        decision_note: row.get(10)?,
        created_at: row.get(11)?,
    })
}

#[cfg(test)]
#[path = "proposal_tests.rs"]
mod tests;
