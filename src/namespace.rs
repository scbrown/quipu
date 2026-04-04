//! Central namespace constants for RDF/OWL/SHACL URIs.
//!
//! All namespace prefixes and commonly-used IRIs are defined here to avoid
//! hardcoded strings scattered across the codebase.

// ── Project namespace ──────────────────────────────────────────

/// Default base namespace for the Aegis ontology.
/// Override via `QuipuConfig::base_ns`.
pub const DEFAULT_BASE_NS: &str = "http://aegis.gastown.local/ontology/";

// ── W3C standard namespaces ────────────────────────────────────

pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
pub const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
pub const PROV: &str = "http://www.w3.org/ns/prov#";
pub const SHACL: &str = "http://www.w3.org/ns/shacl#";

// ── Commonly-used IRIs ─────────────────────────────────────────

pub const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
pub const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

// ── XSD datatype IRIs ──────────────────────────────────────────

pub const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
pub const XSD_LONG: &str = "http://www.w3.org/2001/XMLSchema#long";
pub const XSD_INT: &str = "http://www.w3.org/2001/XMLSchema#int";
pub const XSD_SHORT: &str = "http://www.w3.org/2001/XMLSchema#short";
pub const XSD_BYTE: &str = "http://www.w3.org/2001/XMLSchema#byte";
pub const XSD_NON_NEGATIVE_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#nonNegativeInteger";
pub const XSD_POSITIVE_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#positiveInteger";
pub const XSD_UNSIGNED_LONG: &str = "http://www.w3.org/2001/XMLSchema#unsignedLong";
pub const XSD_UNSIGNED_INT: &str = "http://www.w3.org/2001/XMLSchema#unsignedInt";
pub const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
pub const XSD_FLOAT: &str = "http://www.w3.org/2001/XMLSchema#float";
pub const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
pub const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
