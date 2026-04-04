use thiserror::Error;

/// All errors produced by Quipu.
#[derive(Debug, Error)]
pub enum Error {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("unknown term id: {0}")]
    UnknownTerm(i64),

    #[error("contradiction: entity {entity} attribute {attribute} has overlapping valid-time intervals")]
    Contradiction { entity: i64, attribute: i64 },

    #[error("{0}")]
    InvalidValue(String),
}

pub type Result<T> = std::result::Result<T, Error>;
