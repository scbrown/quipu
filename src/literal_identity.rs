//! RDF term identity is separate from the historical tagged-blob codec.
use crate::{Value, namespace};
use oxrdf::Literal;

/// Preserve literal spelling, including ill-typed lexical forms. Only exact
/// legacy canonical spellings use the compact integer/boolean representations.
pub(crate) fn literal_to_value(lit: &Literal) -> Value {
    if let Some(lang) = lit.language() {
        return Value::Lang {
            lexical: lit.value().into(),
            lang: lang.into(),
        };
    }
    lexical_value(lit.value(), lit.datatype().as_str())
}

fn lexical_value(lexical: &str, datatype: &str) -> Value {
    match datatype {
        namespace::XSD_INTEGER => {
            if let Ok(n) = lexical.parse::<i64>()
                && n.to_string() == lexical
            {
                return Value::Int(n);
            }
        }
        namespace::XSD_BOOLEAN => match lexical {
            "true" => return Value::Bool(true),
            "false" => return Value::Bool(false),
            _ => {}
        },
        namespace::XSD_STRING => return Value::Str(lexical.into()),
        _ => {}
    }
    Value::Typed {
        lexical: lexical.into(),
        datatype: datatype.into(),
    }
}

impl Value {
    /// Lossless identity key for the RDF term this value exports. This is NOT
    /// the storage encoding: never persist this in place of `to_bytes()`.
    /// Legacy Float values retain their old Rust-rendered lexical meaning.
    pub fn term_key(&self) -> Vec<u8> {
        match self {
            Self::Typed { lexical, datatype } => lexical_value(lexical, datatype).to_bytes(),
            Self::Float(value) => Value::Typed {
                lexical: value.to_string(),
                datatype: namespace::XSD_DOUBLE.into(),
            }
            .to_bytes(),
            _ => self.to_bytes(),
        }
    }

    /// Finite list of physical representations for the same exported term.
    /// Store lookup adds any historical NaN payloads; there is no finite list
    /// of all NaN bit patterns that a public API caller could have persisted.
    pub(crate) fn physical_aliases(&self) -> Vec<Vec<u8>> {
        let canonical = Value::from_bytes(&self.term_key()).expect("a Value key is decodable");
        let mut aliases = vec![self.to_bytes(), canonical.to_bytes()];
        let typed = match canonical {
            Self::Int(n) => Some((n.to_string(), namespace::XSD_INTEGER)),
            Self::Bool(b) => Some((b.to_string(), namespace::XSD_BOOLEAN)),
            Self::Str(s) => Some((s, namespace::XSD_STRING)),
            Self::Typed { lexical, datatype } if datatype == namespace::XSD_DOUBLE => {
                if let Ok(f) = lexical.parse::<f64>()
                    && f.to_string() == lexical
                {
                    aliases.push(Self::Float(f).to_bytes());
                }
                None
            }
            _ => None,
        };
        if let Some((lexical, datatype)) = typed {
            aliases.push(
                Self::Typed {
                    lexical,
                    datatype: datatype.into(),
                }
                .to_bytes(),
            );
        }
        aliases.sort_unstable();
        aliases.dedup();
        aliases
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        // Fast common paths, also avoiding allocations for long plain strings.
        match (self, other) {
            (Self::Ref(a), Self::Ref(b)) | (Self::Int(a), Self::Int(b)) => a == b,
            (Self::Str(a), Self::Str(b)) => a == b,
            (Self::Bool(a), Self::Bool(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (
                Self::Typed {
                    lexical: a,
                    datatype: ad,
                },
                Self::Typed {
                    lexical: b,
                    datatype: bd,
                },
            ) => a == b && ad == bd,
            _ => self.term_key() == other.term_key(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codec_and_term_identity_are_separate() {
        let values = [
            Value::Int(1),
            Value::Bool(true),
            Value::Float(-0.0),
            Value::Float(f64::NAN),
        ];
        for value in values {
            let bytes = value.to_bytes();
            assert_eq!(Value::from_bytes(&bytes).unwrap().to_bytes(), bytes);
            let equivalent = match &value {
                Value::Int(n) => Value::Typed {
                    lexical: n.to_string(),
                    datatype: namespace::XSD_INTEGER.into(),
                },
                Value::Bool(b) => Value::Typed {
                    lexical: b.to_string(),
                    datatype: namespace::XSD_BOOLEAN.into(),
                },
                Value::Float(f) => Value::Typed {
                    lexical: f.to_string(),
                    datatype: namespace::XSD_DOUBLE.into(),
                },
                _ => unreachable!(),
            };
            assert_eq!(value, equivalent);
            assert_ne!(bytes, equivalent.to_bytes());
            assert_eq!(value.term_key(), equivalent.term_key());
        }
        assert_ne!(Value::Float(0.0), Value::Float(-0.0));
        assert_ne!(lexical_value("01", namespace::XSD_INTEGER), Value::Int(1));
        assert_ne!(
            lexical_value("1", namespace::XSD_BOOLEAN),
            Value::Bool(true)
        );
    }
}
