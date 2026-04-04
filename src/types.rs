/// Operation type for a fact entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Op {
    /// Retract a previously asserted fact.
    Retract = 0,
    /// Assert a new fact.
    Assert = 1,
}

impl Op {
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Retract),
            1 => Some(Self::Assert),
            _ => None,
        }
    }
}

/// A value stored in the fact log.
///
/// Values are stored as typed blobs with a discriminant tag so round-trip
/// fidelity is preserved without external schema lookups.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// IRI reference (dictionary-encoded term id).
    Ref(i64),
    /// UTF-8 string literal.
    Str(String),
    /// 64-bit signed integer.
    Int(i64),
    /// 64-bit float.
    Float(f64),
    /// Boolean.
    Bool(bool),
    /// Raw bytes.
    Bytes(Vec<u8>),
}

// Tag bytes used as the first byte of the stored BLOB.
const TAG_REF: u8 = 0;
const TAG_STR: u8 = 1;
const TAG_INT: u8 = 2;
const TAG_FLOAT: u8 = 3;
const TAG_BOOL: u8 = 4;
const TAG_BYTES: u8 = 5;

impl Value {
    /// Encode to a tagged blob for `SQLite` storage.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Ref(id) => {
                let mut buf = vec![TAG_REF];
                buf.extend_from_slice(&id.to_le_bytes());
                buf
            }
            Self::Str(s) => {
                let mut buf = vec![TAG_STR];
                buf.extend_from_slice(s.as_bytes());
                buf
            }
            Self::Int(n) => {
                let mut buf = vec![TAG_INT];
                buf.extend_from_slice(&n.to_le_bytes());
                buf
            }
            Self::Float(f) => {
                let mut buf = vec![TAG_FLOAT];
                buf.extend_from_slice(&f.to_le_bytes());
                buf
            }
            Self::Bool(b) => {
                vec![TAG_BOOL, if *b { 1 } else { 0 }]
            }
            Self::Bytes(data) => {
                let mut buf = vec![TAG_BYTES];
                buf.extend_from_slice(data);
                buf
            }
        }
    }

    /// Decode from a tagged blob.
    pub fn from_bytes(data: &[u8]) -> crate::Result<Self> {
        if data.is_empty() {
            return Err(crate::Error::InvalidValue("empty value blob".into()));
        }
        let tag = data[0];
        let payload = &data[1..];
        match tag {
            TAG_REF => {
                let arr: [u8; 8] = payload
                    .try_into()
                    .map_err(|_| crate::Error::InvalidValue("bad ref length".into()))?;
                Ok(Self::Ref(i64::from_le_bytes(arr)))
            }
            TAG_STR => {
                let s = std::str::from_utf8(payload)
                    .map_err(|e| crate::Error::InvalidValue(format!("bad utf8: {e}")))?;
                Ok(Self::Str(s.to_string()))
            }
            TAG_INT => {
                let arr: [u8; 8] = payload
                    .try_into()
                    .map_err(|_| crate::Error::InvalidValue("bad int length".into()))?;
                Ok(Self::Int(i64::from_le_bytes(arr)))
            }
            TAG_FLOAT => {
                let arr: [u8; 8] = payload
                    .try_into()
                    .map_err(|_| crate::Error::InvalidValue("bad float length".into()))?;
                Ok(Self::Float(f64::from_le_bytes(arr)))
            }
            TAG_BOOL => {
                if payload.len() != 1 {
                    return Err(crate::Error::InvalidValue("bad bool length".into()));
                }
                Ok(Self::Bool(payload[0] != 0))
            }
            TAG_BYTES => Ok(Self::Bytes(payload.to_vec())),
            _ => Err(crate::Error::InvalidValue(format!("unknown tag: {tag}"))),
        }
    }
}

/// A term in the dictionary (maps IRI strings to integer IDs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub id: i64,
    pub iri: String,
}

/// A recorded transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub id: i64,
    pub timestamp: String,
    pub actor: Option<String>,
    pub source: Option<String>,
}

/// A single fact in the EAVT log.
#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub entity: i64,
    pub attribute: i64,
    pub value: Value,
    pub tx: i64,
    pub valid_from: String,
    pub valid_to: Option<String>,
    pub op: Op,
}
