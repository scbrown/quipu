use thiserror::Error;

/// All errors produced by Quipu.
#[derive(Debug, Error)]
pub enum Error {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("unknown term id: {0}")]
    UnknownTerm(i64),

    #[error(
        "contradiction: entity {entity} attribute {attribute} has overlapping valid-time intervals"
    )]
    Contradiction { entity: i64, attribute: i64 },

    #[error("{0}")]
    InvalidValue(String),

    #[error("SHACL validation failed: {violations} violation(s)")]
    ValidationFailed {
        violations: usize,
        messages: Vec<String>,
    },

    #[error("policy denied: {0}")]
    PolicyDenied(String),

    #[error(
        "query timeout: exceeded {limit_ms}ms (ran {elapsed_ms}ms) — narrow the query \
         (exact-IRI or rdfs:label lookups, not FILTER(CONTAINS(...)) over unbound patterns) \
         or raise [quipu.search] query_timeout_ms"
    )]
    QueryTimeout { elapsed_ms: u128, limit_ms: u128 },

    #[error(
        "query complexity limit: an intermediate join result exceeded {limit} rows — \
         this query's joins explode (unbound patterns multiplying against each other). \
         Add more selective triple patterns or raise [quipu.search] max_join_rows"
    )]
    QueryComplexity { limit: usize },

    #[error("store error: {0}")]
    Store(String),

    #[error("serialization error: {0}")]
    Serialization(String),
}

pub type Result<T> = std::result::Result<T, Error>;
