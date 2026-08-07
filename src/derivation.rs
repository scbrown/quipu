//! Fact-level derivation methods.
//!
//! `derivedBy` is deliberately a value, not a lattice axis: two executable
//! methods do not meet into a third. Durability is the composable claim; this
//! module records how one concrete statement can be recomputed.

use std::collections::BTreeMap;

use crate::error::{Error, Result};
use crate::namespace::{
    QUIPU_DERIVATION_PARAMS, QUIPU_DERIVATION_QUERY, QUIPU_DERIVATION_SYSTEM, QUIPU_DERIVED_BY,
    RDF_OBJECT, RDF_PREDICATE, RDF_STATEMENT, RDF_SUBJECT, RDF_TYPE,
};
use crate::store::{Datum, Store};
use crate::types::{Fact, Op, Value};

/// An executable recipe for recomputing one fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivationMethod {
    /// Adapter/system name, such as `prometheus` or `quipu-query`.
    pub system: String,
    /// Query or command understood by `system`.
    pub query: String,
    /// Canonically ordered string parameters passed to the query.
    pub params: BTreeMap<String, String>,
}

/// The result of executing a fact's declared method.
#[derive(Debug, Clone, PartialEq)]
pub struct Rederivation {
    /// Value currently asserted by the fact.
    pub expected: Value,
    /// Value returned by executing the method.
    pub derived: Value,
    /// Whether `derived` exactly matches `expected`.
    pub matches: bool,
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

impl Store {
    fn fact_key(&self, fact: &Fact) -> Result<String> {
        let entity = self.resolve(fact.entity)?;
        let predicate = self.resolve(fact.attribute)?;
        let object = match &fact.value {
            Value::Ref(id) => format!("ref:{}", self.resolve(*id)?),
            other => format!("literal:{other:?}"),
        };
        Ok(format!("{entity}|{predicate}|{object}"))
    }

    fn statement_iri(&self, fact: &Fact) -> Result<String> {
        Ok(format!(
            "urn:quipu:statement:{:016x}",
            fnv1a64(self.fact_key(fact)?.as_bytes())
        ))
    }

    /// Attach one explicit derivation method to an active fact.
    pub fn set_fact_derivation(
        &mut self,
        fact: &Fact,
        method: &DerivationMethod,
        timestamp: &str,
        actor: Option<&str>,
    ) -> Result<i64> {
        if method.system.trim().is_empty() || method.query.trim().is_empty() {
            return Err(Error::InvalidValue(
                "a derivation method requires non-empty system and query".into(),
            ));
        }
        let active = self
            .entity_facts(fact.entity)?
            .into_iter()
            .any(|f| f.attribute == fact.attribute && f.value == fact.value && f.op == Op::Assert);
        if !active {
            return Err(Error::InvalidValue(
                "cannot attach a derivation method to a fact that is not active".into(),
            ));
        }

        let stmt = self.intern(&self.statement_iri(fact)?)?;
        let params = serde_json::to_string(&method.params)
            .map_err(|e| Error::InvalidValue(format!("invalid derivation params: {e}")))?;
        let method_key = format!("{}|{}|{}", method.system, method.query, params);
        let method_id = self.intern(&format!(
            "urn:quipu:derivation:{:016x}",
            fnv1a64(method_key.as_bytes())
        ))?;
        let datums = vec![
            Datum {
                entity: stmt,
                attribute: self.intern(RDF_TYPE)?,
                value: Value::Ref(self.intern(RDF_STATEMENT)?),
                valid_from: timestamp.into(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: stmt,
                attribute: self.intern(RDF_SUBJECT)?,
                value: Value::Ref(fact.entity),
                valid_from: timestamp.into(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: stmt,
                attribute: self.intern(RDF_PREDICATE)?,
                value: Value::Ref(fact.attribute),
                valid_from: timestamp.into(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: stmt,
                attribute: self.intern(RDF_OBJECT)?,
                value: fact.value.clone(),
                valid_from: timestamp.into(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: stmt,
                attribute: self.intern(QUIPU_DERIVED_BY)?,
                value: Value::Ref(method_id),
                valid_from: timestamp.into(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: method_id,
                attribute: self.intern(QUIPU_DERIVATION_SYSTEM)?,
                value: Value::Str(method.system.clone()),
                valid_from: timestamp.into(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: method_id,
                attribute: self.intern(QUIPU_DERIVATION_QUERY)?,
                value: Value::Str(method.query.clone()),
                valid_from: timestamp.into(),
                valid_to: None,
                op: Op::Assert,
            },
            Datum {
                entity: method_id,
                attribute: self.intern(QUIPU_DERIVATION_PARAMS)?,
                value: Value::Str(params),
                valid_from: timestamp.into(),
                valid_to: None,
                op: Op::Assert,
            },
        ];
        self.transact(&datums, timestamp, actor, Some("fact-derivation"))
    }

    /// Read a fact's method. No declaration returns `None`, never a default.
    pub fn fact_derivation(&self, fact: &Fact) -> Result<Option<DerivationMethod>> {
        let Some(stmt) = self.lookup(&self.statement_iri(fact)?)? else {
            return Ok(None);
        };
        let derived_by = self.lookup(QUIPU_DERIVED_BY)?;
        let Some(method_id) = self.entity_facts(stmt)?.into_iter().find_map(|f| {
            (Some(f.attribute) == derived_by)
                .then_some(f.value)
                .and_then(|v| match v {
                    Value::Ref(id) => Some(id),
                    _ => None,
                })
        }) else {
            return Ok(None);
        };

        let sys = self.lookup(QUIPU_DERIVATION_SYSTEM)?;
        let query = self.lookup(QUIPU_DERIVATION_QUERY)?;
        let params = self.lookup(QUIPU_DERIVATION_PARAMS)?;
        let mut system = None;
        let mut query_text = None;
        let mut params_text = None;
        for f in self.entity_facts(method_id)? {
            if Some(f.attribute) == sys {
                if let Value::Str(v) = f.value {
                    system = Some(v);
                }
            } else if Some(f.attribute) == query {
                if let Value::Str(v) = f.value {
                    query_text = Some(v);
                }
            } else if Some(f.attribute) == params {
                if let Value::Str(v) = f.value {
                    params_text = Some(v);
                }
            }
        }
        let (Some(system), Some(query), Some(params_text)) = (system, query_text, params_text)
        else {
            return Err(Error::InvalidValue(
                "incomplete derivation method in graph".into(),
            ));
        };
        let params = serde_json::from_str(&params_text)
            .map_err(|e| Error::InvalidValue(format!("invalid derivation params JSON: {e}")))?;
        Ok(Some(DerivationMethod {
            system,
            query,
            params,
        }))
    }

    /// Execute the declared method through a caller-owned system adapter.
    pub fn rederive_fact<F>(&self, fact: &Fact, mut execute: F) -> Result<Option<Rederivation>>
    where
        F: FnMut(&DerivationMethod) -> Result<Value>,
    {
        let Some(method) = self.fact_derivation(fact)? else {
            return Ok(None);
        };
        let derived = execute(&method)?;
        Ok(Some(Rederivation {
            matches: derived == fact.value,
            expected: fact.value.clone(),
            derived,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_method_rederives_and_absence_never_defaults() {
        let mut store = Store::open_in_memory().unwrap();
        let e = store.intern("https://example.test/metric").unwrap();
        let a = store.intern("https://example.test/value").unwrap();
        store
            .transact(
                &[Datum {
                    entity: e,
                    attribute: a,
                    value: Value::Int(42),
                    valid_from: "2026-08-07T00:00:00Z".into(),
                    valid_to: None,
                    op: Op::Assert,
                }],
                "2026-08-07T00:00:00Z",
                None,
                Some("test"),
            )
            .unwrap();
        let fact = store.entity_facts(e).unwrap().remove(0);
        assert_eq!(store.fact_derivation(&fact).unwrap(), None);
        assert!(
            store
                .rederive_fact(&fact, |_| Ok(Value::Int(0)))
                .unwrap()
                .is_none()
        );

        let method = DerivationMethod {
            system: "prometheus".into(),
            query: "answer".into(),
            params: BTreeMap::from([("instance".into(), "test".into())]),
        };
        store
            .set_fact_derivation(&fact, &method, "2026-08-07T00:01:00Z", None)
            .unwrap();
        assert_eq!(store.fact_derivation(&fact).unwrap(), Some(method.clone()));
        let result = store
            .rederive_fact(&fact, |m| {
                assert_eq!(m.system, "prometheus");
                Ok(Value::Int(42))
            })
            .unwrap()
            .unwrap();
        assert!(result.matches);
    }
}
