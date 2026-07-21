/// Operation type for a fact entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Op {
    /// Retract a previously asserted fact.
    Retract = 0,
    /// Assert a new fact.
    Assert = 1,
    /// Tombstone: mark a specific `(e,a,v)` triple ABSENT in an overlay's
    /// composed view (aegis-g1al / quipu #36). Unlike Retract — which closes an
    /// assertion in the same graph — a Tombstone hides a triple that lives in a
    /// LOWER layer (the parent-branch root) without mutating it. Meaningful only
    /// inside an overlay graph; the composed view excludes any root triple the
    /// overlay tombstones. Overlay-class, so it never reaches the committed
    /// bitemporal/reactive paths.
    Tombstone = 2,
}

impl Op {
    pub fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Retract),
            1 => Some(Self::Assert),
            2 => Some(Self::Tombstone),
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
    /// Language-tagged literal (`rdf:langString`): the LEXICAL value and the
    /// BCP47 tag, stored separately.
    ///
    /// This variant exists because the tag used to be concatenated into the
    /// lexical form (`"hello"@en` -> `Str("hello@en")`), which is irreversible
    /// corruption: the plain string `hello@en` collapsed to the same value
    /// (aegis-fmyi). Never reconstruct a lang tag by splitting a `Str` on `@`.
    Lang {
        /// The literal's lexical form, WITHOUT the tag: `"hello"`, never
        /// `"hello@en"`.
        lexical: String,
        /// BCP47 language tag, without the leading `@`: `en`, `en-GB`.
        lang: String,
    },
    /// Literal carrying an explicit datatype IRI that has no fast-path variant
    /// (`xsd:date`, `xsd:dateTime`, `xsd:decimal`, integer subtypes, and any
    /// custom datatype). The lexical form is preserved verbatim so the term
    /// round-trips exactly.
    Typed {
        /// The literal's lexical form, preserved verbatim (no reparsing, no
        /// canonicalization) so the term round-trips byte-for-byte.
        lexical: String,
        /// The datatype IRI in full, e.g.
        /// `http://www.w3.org/2001/XMLSchema#date`.
        datatype: String,
    },
}

// Tag bytes used as the first byte of the stored BLOB.
const TAG_REF: u8 = 0;
const TAG_STR: u8 = 1;
const TAG_INT: u8 = 2;
const TAG_FLOAT: u8 = 3;
const TAG_BOOL: u8 = 4;
const TAG_BYTES: u8 = 5;
const TAG_LANG: u8 = 6;
const TAG_TYPED: u8 = 7;

/// Encode `{prefix, lexical}` as `[u16 LE prefix len][prefix][lexical]`.
///
/// The prefix (language tag / datatype IRI) is length-delimited so the lexical
/// form can contain any byte sequence — including `@` — without ambiguity.
fn encode_prefixed(tag: u8, prefix: &str, lexical: &str) -> Vec<u8> {
    let plen = u16::try_from(prefix.len()).unwrap_or(u16::MAX);
    let mut buf = Vec::with_capacity(3 + prefix.len() + lexical.len());
    buf.push(tag);
    buf.extend_from_slice(&plen.to_le_bytes());
    buf.extend_from_slice(&prefix.as_bytes()[..plen as usize]);
    buf.extend_from_slice(lexical.as_bytes());
    buf
}

fn decode_prefixed(payload: &[u8], what: &str) -> crate::Result<(String, String)> {
    if payload.len() < 2 {
        return Err(crate::Error::InvalidValue(format!(
            "truncated {what} value"
        )));
    }
    let plen = u16::from_le_bytes([payload[0], payload[1]]) as usize;
    if payload.len() < 2 + plen {
        return Err(crate::Error::InvalidValue(format!(
            "truncated {what} prefix"
        )));
    }
    let prefix = std::str::from_utf8(&payload[2..2 + plen])
        .map_err(|e| crate::Error::InvalidValue(format!("bad utf8 in {what} prefix: {e}")))?;
    let lexical = std::str::from_utf8(&payload[2 + plen..])
        .map_err(|e| crate::Error::InvalidValue(format!("bad utf8 in {what} lexical: {e}")))?;
    Ok((prefix.to_string(), lexical.to_string()))
}

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
            Self::Lang { lexical, lang } => encode_prefixed(TAG_LANG, lang, lexical),
            Self::Typed { lexical, datatype } => encode_prefixed(TAG_TYPED, datatype, lexical),
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
            TAG_LANG => {
                let (lang, lexical) = decode_prefixed(payload, "lang")?;
                Ok(Self::Lang { lexical, lang })
            }
            TAG_TYPED => {
                let (datatype, lexical) = decode_prefixed(payload, "typed")?;
                Ok(Self::Typed { lexical, datatype })
            }
            _ => Err(crate::Error::InvalidValue(format!("unknown tag: {tag}"))),
        }
    }

    /// The datatype IRI this value serializes as, if it is a literal with one.
    ///
    /// `None` for `Ref` (not a literal) and for plain strings (`xsd:string` is
    /// implicit in RDF 1.1) and lang literals (their datatype is
    /// `rdf:langString`, implied by the tag).
    pub fn datatype(&self) -> Option<&str> {
        match self {
            Self::Typed { datatype, .. } => Some(datatype),
            Self::Int(_) => Some(crate::namespace::XSD_INTEGER),
            Self::Float(_) => Some(crate::namespace::XSD_DOUBLE),
            Self::Bool(_) => Some(crate::namespace::XSD_BOOLEAN),
            _ => None,
        }
    }

    /// The literal's LEXICAL form — without any language tag or datatype
    /// suffix. `None` for values that are not literals.
    ///
    /// `Lang { lexical: "hello", lang: "en" }.as_lexical()` is `"hello"`. That
    /// it once was `"hello@en"` is the whole of aegis-fmyi.
    pub fn as_lexical(&self) -> Option<&str> {
        match self {
            Self::Str(s) | Self::Lang { lexical: s, .. } | Self::Typed { lexical: s, .. } => {
                Some(s)
            }
            _ => None,
        }
    }

    /// The language tag, if this is a language-tagged literal.
    pub fn language(&self) -> Option<&str> {
        match self {
            Self::Lang { lang, .. } => Some(lang),
            _ => None,
        }
    }

    /// Numeric view of a value for ordering/aggregation, including numeric
    /// `Typed` literals (integer subtypes, `xsd:float`, `xsd:decimal`) which
    /// keep their datatype rather than collapsing into `Int`/`Float`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(n) => Some(*n as f64),
            Self::Float(f) => Some(*f),
            Self::Typed { lexical, datatype }
                if crate::namespace::is_numeric_datatype(datatype) =>
            {
                lexical.parse().ok()
            }
            _ => None,
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
