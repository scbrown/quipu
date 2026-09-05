//! The SSSOM mapping set: the alignment artifact, and its mandatory TSV form.
//!
//! [SSSOM](https://mapping-commons.github.io/sssom/) is the interchange format
//! for "these two concepts are the same thing" — see
//! `docs/design/cross-graph-alignment.md` Decision 1 for why quipu adopts it
//! rather than inventing a table.
//!
//! Three properties of the standard are the reason it is here, and each one is
//! a thing we would otherwise have got wrong:
//!
//! * **Negative mappings are in the model.** `predicate_modifier: Not` says
//!   "these are NOT the same", which is the operator's most durable output —
//!   the one judgement a matcher can never re-derive — and it has somewhere to
//!   live instead of being re-proposed forever.
//! * **The justification is a controlled vocabulary** (`semapv:`), not free
//!   text, so *why a mapping exists* survives the session that produced it.
//! * **TSV is the MUST serialisation.** An implementation without it is not an
//!   SSSOM implementation, and `sssom-py` interop is most of the point.
//!
//! ## What "unreviewed" means here
//!
//! `author_id` absent marks a row the operator has not decided. That is the
//! whole state machine: `propose` writes rows with no author, `decide` fills in
//! the author, the date, and the operator's own justification
//! (`semapv:ManualMappingCuration` — *the recorded justification is the
//! operator's, not the matcher's*), and `apply` derives knots only from
//! authored, non-negated rows. A skipped row stays unauthored and comes back
//! next time, which is why skip and reject must never be collapsed.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// `sssom:` — slot IRIs are this namespace plus the slot name.
pub const SSSOM_NS: &str = "https://w3id.org/sssom/";
/// `semapv:` — the mapping-justification vocabulary.
pub const SEMAPV_NS: &str = "https://w3id.org/semapv/vocab/";
/// The predicate an accepted mapping derives a knot from.
pub const OWL_SAME_AS: &str = "http://www.w3.org/2002/07/owl#sameAs";

/// The SSSOM slots quipu writes, in the column order it emits them.
///
/// Order is fixed rather than derived so that `propose` run twice is
/// byte-identical (acceptance criterion 2). A column set that reshuffles
/// between runs cannot be reviewed in a diff.
const COLUMNS: [&str; 11] = [
    "subject_id",
    "subject_label",
    "predicate_id",
    "object_id",
    "object_label",
    "mapping_justification",
    "predicate_modifier",
    "confidence",
    "author_id",
    // quipu extension slots. SSSOM permits extra columns and `sssom-py` carries
    // them through, so a declined candidate travels with the set without
    // pretending to be a mapping. See `Review` for why it cannot be an
    // ordinary SSSOM row.
    "quipu_review",
    "quipu_reviewed_by",
];

/// How a mapping came to be proposed, as a `semapv:` term.
///
/// The mapping from quipu's `matched_on` strings is in
/// [`Justification::from_matched_on`]; every term here was verified present in
/// the vocabulary (design doc, Decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Justification {
    /// `canonical_name:exact` — an exact lexical match.
    LexicalMatching,
    /// `canonical_name:jaro_winkler:<n>` — lexical similarity over a threshold.
    LexicalSimilarityThresholdMatching,
    /// `embedding:<n>` — vector similarity.
    EmbeddingBasedMatching,
    /// Several signals combined by a link specification.
    CompositeMatching,
    /// A human decided. This is what `decide` records, over whatever the
    /// matcher said.
    ManualMappingCuration,
}

impl Justification {
    /// The `semapv:` CURIE, as written into the TSV.
    #[must_use]
    pub const fn curie(self) -> &'static str {
        match self {
            Self::LexicalMatching => "semapv:LexicalMatching",
            Self::LexicalSimilarityThresholdMatching => "semapv:LexicalSimilarityThresholdMatching",
            Self::EmbeddingBasedMatching => "semapv:EmbeddingBasedMatching",
            Self::CompositeMatching => "semapv:CompositeMatching",
            Self::ManualMappingCuration => "semapv:ManualMappingCuration",
        }
    }

    /// Parse a `semapv:` CURIE or full IRI.
    ///
    /// # Errors
    /// The term is not one quipu emits.
    pub fn parse(value: &str) -> Result<Self> {
        let bare = value
            .strip_prefix("semapv:")
            .or_else(|| value.strip_prefix(SEMAPV_NS))
            .unwrap_or(value);
        match bare {
            "LexicalMatching" => Ok(Self::LexicalMatching),
            "LexicalSimilarityThresholdMatching" => Ok(Self::LexicalSimilarityThresholdMatching),
            "EmbeddingBasedMatching" => Ok(Self::EmbeddingBasedMatching),
            "CompositeMatching" => Ok(Self::CompositeMatching),
            "ManualMappingCuration" => Ok(Self::ManualMappingCuration),
            other => Err(Error::InvalidValue(format!(
                "unknown mapping_justification {other:?}; expected a semapv term quipu emits"
            ))),
        }
    }

    /// The `semapv:` term for one of `resolve_entity`'s `matched_on` strings.
    ///
    /// `matched_on` carries its score inline (`canonical_name:jaro_winkler:0.95`),
    /// so this matches on the prefix rather than the whole string.
    #[must_use]
    pub fn from_matched_on(matched_on: &str) -> Self {
        if matched_on == "canonical_name:exact" {
            Self::LexicalMatching
        } else if matched_on.starts_with("canonical_name:jaro_winkler") {
            Self::LexicalSimilarityThresholdMatching
        } else if matched_on.starts_with("embedding:") {
            Self::EmbeddingBasedMatching
        } else {
            Self::CompositeMatching
        }
    }
}

/// What the operator did with a candidate, where that is NOT an assertion.
///
/// ## Why a reject needs splitting in two (wu, aegis-sosiaa review)
///
/// `owl:sameAs` asserts identity; `quipu:distinctFrom` asserts NON-identity.
/// Both are claims about the world. But a reject in a review loop usually means
/// *"not enough evidence"*, not *"these are definitely different things"* —
/// and deriving `distinctFrom` from a bare reject converts absence of evidence
/// into an assertion of difference. That is the exact mirror of the
/// `skos:closeMatch` error [`Mapping::derives_knot`] avoids: a knot asserts
/// identity and closeMatch does not.
///
/// The consequence is asymmetric in the dangerous direction. A wrong
/// `owl:sameAs` merges two entities and the merged thing looks wrong to the
/// next reader. A wrong `distinctFrom` suppresses the pair **everywhere,
/// forever, and invisibly by construction** — the system's response to it is to
/// stop mentioning the candidate, so nobody is ever shown the mistake.
///
/// So `Declined` is deliberately NOT an SSSOM assertion: `author_id` stays
/// absent, because an authored row is one an SSSOM consumer may read as curated
/// truth, and nothing was asserted here. The review state rides in quipu
/// extension slots instead, where it suppresses re-proposal and derives
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Review {
    /// Seen, and set aside without a claim either way. Suppresses re-proposal;
    /// asserts nothing; derives nothing.
    Declined,
}

impl Review {
    /// The value written into the `quipu_review` column.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Declined => "declined",
        }
    }

    /// Parse the `quipu_review` column.
    ///
    /// # Errors
    /// The value is not one quipu writes. Refused rather than ignored: reading
    /// an unknown review state as "no review" puts a declined candidate back
    /// into the proposal stream.
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "declined" => Ok(Self::Declined),
            other => Err(Error::InvalidValue(format!(
                "unknown quipu_review {other:?}; expected \"declined\""
            ))),
        }
    }
}

/// One row of a mapping set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    /// The IRI on the left. Required by SSSOM.
    pub subject_id: String,
    /// The left concept's label, carried for the operator's benefit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_label: Option<String>,
    /// The mapping predicate. Required by SSSOM.
    ///
    /// Kept as a free string rather than an enum: SSSOM allows any predicate,
    /// and a hand-edited `skos:closeMatch` must round-trip honestly even though
    /// v1 derives knots only from `owl:sameAs`.
    pub predicate_id: String,
    /// The IRI on the right. Required by SSSOM.
    pub object_id: String,
    /// The right concept's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_label: Option<String>,
    /// Why this mapping exists. Required by SSSOM.
    pub mapping_justification: Justification,
    /// `Some(true)` for a NEGATIVE mapping (`predicate_modifier: Not`).
    ///
    /// A negated row derives nothing. Its entire job is to suppress a future
    /// proposal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate_modifier_not: Option<bool>,
    /// The matcher's score, where one produced this row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Who ASSERTED. **Absence is what marks a row un-asserted.**
    ///
    /// A declined row leaves this absent on purpose — see [`Review`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    /// Reviewed but not asserted. Suppresses re-proposal, derives nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quipu_review: Option<Review>,
    /// Who reviewed, for a row that carries [`Review`] rather than an author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quipu_reviewed_by: Option<String>,
}

impl Mapping {
    /// Did an operator ASSERT something here — an identity or a difference?
    ///
    /// This is the SSSOM question. It is deliberately narrower than
    /// [`Mapping::is_reviewed`]: a declined row was ruled on and asserts
    /// nothing.
    #[must_use]
    pub fn is_decided(&self) -> bool {
        self.author_id.is_some()
    }

    /// Has an operator SEEN this pair — asserted or declined?
    ///
    /// This is the suppression question, and the one `propose` asks. Splitting
    /// it from [`Mapping::is_decided`] is the whole point of [`Review`]: a
    /// candidate can be taken out of the review queue without a claim being
    /// made about the world.
    #[must_use]
    pub fn is_reviewed(&self) -> bool {
        self.is_decided() || self.quipu_review.is_some()
    }

    /// Is this a NOT mapping — an explicit "these are different things"?
    #[must_use]
    pub fn is_negated(&self) -> bool {
        self.predicate_modifier_not.unwrap_or(false)
    }

    /// Should this row derive an `owl:sameAs` knot?
    ///
    /// Decided, not negated, and `owl:sameAs` specifically — a `skos:closeMatch`
    /// says the concepts are NEAR, not identical, so deriving `sameAs` from one
    /// would assert something the operator did not.
    #[must_use]
    pub fn derives_knot(&self) -> bool {
        self.is_decided()
            && !self.is_negated()
            && (self.predicate_id == OWL_SAME_AS || self.predicate_id == "owl:sameAs")
    }

    /// Should this row derive a `quipu:distinctFrom` — an assertion that the
    /// two concepts are NOT the same?
    ///
    /// Only an ASSERTED negative does. A declined row must not, because
    /// "not enough evidence" is not "definitely different", and the derived
    /// triple would suppress the pair everywhere while nobody is ever shown it
    /// again. See [`Review`].
    #[must_use]
    pub fn derives_distinct_from(&self) -> bool {
        self.is_decided() && self.is_negated()
    }

    /// The unordered identity of the pair, for de-duplication and for matching
    /// a proposal against an existing decision.
    ///
    /// Unordered because "A is the same as B" and "B is the same as A" are one
    /// judgement, and a rejection recorded in one direction must suppress a
    /// proposal generated in the other.
    #[must_use]
    pub fn pair_key(&self) -> (String, String) {
        if self.subject_id <= self.object_id {
            (self.subject_id.clone(), self.object_id.clone())
        } else {
            (self.object_id.clone(), self.subject_id.clone())
        }
    }
}

/// A set of mappings plus the metadata SSSOM carries in its YAML header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MappingSet {
    /// `mapping_set_id` — the set's own IRI.
    pub mapping_set_id: String,
    /// CURIE prefix declarations, written into the header.
    #[serde(default)]
    pub curie_map: BTreeMap<String, String>,
    /// The rows.
    #[serde(default)]
    pub mappings: Vec<Mapping>,
}

impl MappingSet {
    /// A set with the prefixes quipu always declares.
    #[must_use]
    pub fn new(mapping_set_id: impl Into<String>) -> Self {
        let mut curie_map = BTreeMap::new();
        curie_map.insert("owl".into(), "http://www.w3.org/2002/07/owl#".into());
        curie_map.insert("semapv".into(), SEMAPV_NS.into());
        curie_map.insert("skos".into(), "http://www.w3.org/2004/02/skos/core#".into());
        Self {
            mapping_set_id: mapping_set_id.into(),
            curie_map,
            mappings: Vec::new(),
        }
    }

    /// Sort the rows into the canonical order `propose` emits.
    ///
    /// Deterministic output is acceptance criterion 2, and it has to be a
    /// property of the SET rather than of the caller: two callers that build
    /// the same mappings in different orders must serialise identically.
    pub fn sort(&mut self) {
        self.mappings.sort_by(|a, b| {
            a.subject_id
                .cmp(&b.subject_id)
                .then_with(|| a.object_id.cmp(&b.object_id))
                .then_with(|| a.predicate_id.cmp(&b.predicate_id))
        });
    }

    /// Every pair an operator has SEEN, asserted or declined.
    ///
    /// This is what `propose` consults so a reviewed pair is not offered again
    /// (acceptance criterion 3). It is keyed on review rather than assertion
    /// because a declined candidate must also stop coming back — that is the
    /// point of declining it.
    #[must_use]
    pub fn reviewed(&self) -> BTreeMap<(String, String), ()> {
        self.mappings
            .iter()
            .filter(|m| m.is_reviewed())
            .map(|m| (m.pair_key(), ()))
            .collect()
    }

    /// The pairs an operator asserted to be DIFFERENT, which are the only rows
    /// that derive a `quipu:distinctFrom`.
    #[must_use]
    pub fn asserted_different(&self) -> Vec<&Mapping> {
        self.mappings
            .iter()
            .filter(|m| m.derives_distinct_from())
            .collect()
    }
}

// ---------------------------------------------------------------- SSSOM/TSV
//
// The mandatory serialisation: a YAML metadata header, each line prefixed `#`,
// then a TSV table. Written by hand rather than through a YAML/CSV crate
// because the shape is fixed and small, and because the exact bytes matter —
// determinism is an acceptance criterion, and a library that reorders map keys
// or quotes opportunistically would put that outside our control.

impl MappingSet {
    /// Serialise to SSSOM/TSV.
    ///
    /// # Errors
    /// A field contains a tab or newline, which TSV cannot represent and SSSOM
    /// gives no escape for. Refusing is the honest option: silently stripping
    /// the character would change an IRI or a label into a different one.
    pub fn to_tsv(&self) -> Result<String> {
        let mut out = String::new();
        out.push_str("#curie_map:\n");
        for (prefix, expansion) in &self.curie_map {
            let _ = writeln!(out, "#  {prefix}: \"{expansion}\"");
        }
        let _ = writeln!(out, "#mapping_set_id: \"{}\"", self.mapping_set_id);
        out.push_str(&COLUMNS.join("\t"));
        out.push('\n');

        for m in &self.mappings {
            let confidence = m.confidence.map(|c| format!("{c:.3}")).unwrap_or_default();
            let modifier = if m.is_negated() { "Not" } else { "" };
            let cells = [
                m.subject_id.as_str(),
                m.subject_label.as_deref().unwrap_or(""),
                m.predicate_id.as_str(),
                m.object_id.as_str(),
                m.object_label.as_deref().unwrap_or(""),
                m.mapping_justification.curie(),
                modifier,
                confidence.as_str(),
                m.author_id.as_deref().unwrap_or(""),
                m.quipu_review.map_or("", Review::as_str),
                m.quipu_reviewed_by.as_deref().unwrap_or(""),
            ];
            for (column, cell) in COLUMNS.iter().zip(cells.iter()) {
                if cell.contains('\t') || cell.contains('\n') {
                    return Err(Error::InvalidValue(format!(
                        "SSSOM/TSV cannot represent a tab or newline in {column}: {cell:?}"
                    )));
                }
            }
            out.push_str(&cells.join("\t"));
            out.push('\n');
        }
        Ok(out)
    }

    /// Parse SSSOM/TSV.
    ///
    /// Reads the columns by NAME from the header row, not by position, so a set
    /// a human reordered by hand — or one `sssom-py` wrote with its own column
    /// order — still loads. Unknown columns are ignored rather than refused:
    /// SSSOM has many slots quipu does not write, and a set that round-trips
    /// through other tooling should not become unreadable here.
    ///
    /// # Errors
    /// A required slot is missing, or a value does not parse.
    pub fn from_tsv(text: &str) -> Result<Self> {
        let mut curie_map = BTreeMap::new();
        let mut mapping_set_id = String::new();
        let mut lines = text.lines().peekable();

        while let Some(line) = lines.peek() {
            let Some(rest) = line.strip_prefix('#') else {
                break;
            };
            let trimmed = rest.trim();
            if let Some(value) = trimmed.strip_prefix("mapping_set_id:") {
                mapping_set_id = value.trim().trim_matches('"').to_string();
            } else if let Some((prefix, expansion)) = trimmed.split_once(':')
                && !prefix.is_empty()
                && prefix != "curie_map"
            {
                curie_map.insert(
                    prefix.trim().to_string(),
                    expansion.trim().trim_matches('"').to_string(),
                );
            }
            lines.next();
        }

        let header = lines
            .next()
            .ok_or_else(|| Error::InvalidValue("SSSOM/TSV has no header row".into()))?;
        let index: BTreeMap<&str, usize> = header
            .split('\t')
            .enumerate()
            .map(|(i, name)| (name.trim(), i))
            .collect();
        for required in [
            "subject_id",
            "object_id",
            "predicate_id",
            "mapping_justification",
        ] {
            if !index.contains_key(required) {
                return Err(Error::InvalidValue(format!(
                    "SSSOM/TSV is missing the required column {required}"
                )));
            }
        }

        let mut mappings = Vec::new();
        for (row, line) in lines.enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let cells: Vec<&str> = line.split('\t').collect();
            let get = |name: &str| -> &str {
                index
                    .get(name)
                    .and_then(|i| cells.get(*i))
                    .copied()
                    .unwrap_or("")
                    .trim()
            };
            let some =
                |value: &str| -> Option<String> { (!value.is_empty()).then(|| value.to_string()) };
            let confidence = match get("confidence") {
                "" => None,
                raw => Some(raw.parse::<f64>().map_err(|e| {
                    Error::InvalidValue(format!(
                        "SSSOM/TSV row {}: confidence {raw:?}: {e}",
                        row + 1
                    ))
                })?),
            };
            // Only the literal `Not` is a negation. An unrecognised modifier is
            // refused rather than treated as absent: reading an unknown value
            // as "no modifier" would turn a rejection into a proposal.
            let predicate_modifier_not = match get("predicate_modifier") {
                "" => None,
                "Not" => Some(true),
                other => {
                    return Err(Error::InvalidValue(format!(
                        "SSSOM/TSV row {}: predicate_modifier {other:?}; only \"Not\" is supported",
                        row + 1
                    )));
                }
            };
            mappings.push(Mapping {
                subject_id: get("subject_id").to_string(),
                subject_label: some(get("subject_label")),
                predicate_id: get("predicate_id").to_string(),
                object_id: get("object_id").to_string(),
                object_label: some(get("object_label")),
                mapping_justification: Justification::parse(get("mapping_justification"))?,
                predicate_modifier_not,
                confidence,
                author_id: some(get("author_id")),
                quipu_review: match get("quipu_review") {
                    "" => None,
                    raw => Some(Review::parse(raw).map_err(|e| {
                        Error::InvalidValue(format!("SSSOM/TSV row {}: {e}", row + 1))
                    })?),
                },
                quipu_reviewed_by: some(get("quipu_reviewed_by")),
            });
        }

        Ok(Self {
            mapping_set_id,
            curie_map,
            mappings,
        })
    }
}
