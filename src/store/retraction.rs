//! Episode-retraction policy and outcome types.

use crate::types::Fact;

/// What episode-scoped retraction does when removing an episode's facts would
/// strip a node's identity while leaving it referenced by other episodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrphanPolicy {
    /// Keep identity facts alive for any entity with surviving references.
    #[default]
    Preserve,
    /// Refuse the whole retraction if it would orphan any identity.
    Refuse,
    /// Apply legacy strict episode scope, while reporting resulting orphans.
    Allow,
}

impl OrphanPolicy {
    /// Parse a policy name (`preserve` | `refuse` | `allow`). Case-insensitive.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "preserve" => Some(Self::Preserve),
            "refuse" => Some(Self::Refuse),
            "allow" => Some(Self::Allow),
            _ => None,
        }
    }

    /// The policy's canonical name, as accepted by [`OrphanPolicy::parse`].
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::Refuse => "refuse",
            Self::Allow => "allow",
        }
    }
}

/// An entity whose identity the retraction would strip while retaining it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityOrphan {
    /// The entity term id.
    pub entity: i64,
    /// The episode declared this entity's only `rdfs:label`.
    pub lost_label: bool,
    /// The episode declared this entity's only `rdf:type`.
    pub lost_type: bool,
}

/// Result of an episode-scoped retraction.
#[derive(Debug, Clone)]
pub struct RetractEpisodeOutcome {
    /// The retraction transaction, or [`crate::episode::NOOP_TX`] for a no-op.
    pub tx_id: i64,
    /// Facts actually closed by this retraction.
    pub retracted: Vec<Fact>,
    /// Identity facts deliberately left active under `Preserve`.
    pub preserved_identity: Vec<Fact>,
    /// Entities whose identity was at risk.
    pub orphans: Vec<IdentityOrphan>,
    /// The policy that was applied.
    pub policy: OrphanPolicy,
}
