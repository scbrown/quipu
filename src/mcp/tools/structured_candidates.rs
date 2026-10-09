//! Explicit structured candidate selection before ranking. Never post-filter
//! an oversampled global top-K. SQLite-only until other backends implement an
//! equally complete candidate contract; no silent fallback.
use super::structured_syntax::{self, Expr};
use crate::sparql::{self, TemporalContext};
use crate::{Error, Result, Store, Value};
use std::collections::HashSet;
pub(super) const CAP: usize = 4096;
const LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidValue(message.into())
}
fn literal(s: &str) -> String {
    serde_json::to_string(s).expect("string serializes")
}
fn iri(s: &str) -> Result<String> {
    if !s.contains(':')
        || s.chars()
            .any(|c| c.is_control() || c.is_whitespace() || "<>\"{}\\".contains(c))
    {
        return Err(invalid(format!("unsafe or unresolved IRI {s:?}")));
    }
    Ok(format!("<{s}>"))
}
pub(super) struct Context<'a> {
    pub(super) store: &'a Store,
    pub(super) at: Option<&'a str>,
}
impl Context<'_> {
    fn select(&self, pattern: &str, variable: &str) -> Result<HashSet<String>> {
        let text = format!(
            "SELECT DISTINCT ?{variable} WHERE {{ {pattern} }} LIMIT {}",
            CAP + 1
        );
        let result = sparql::query_temporal(
            self.store,
            &text,
            &TemporalContext {
                valid_at: self.at.map(str::to_owned),
                row_cap: Some(CAP * 8),
                ..Default::default()
            },
        )?;
        if result.rows().len() > CAP {
            return Err(invalid(
                "structured candidate cap 4096 exceeded; narrow the expression",
            ));
        }
        result
            .rows()
            .iter()
            .filter_map(|r| r.get(variable))
            .map(|v| match v {
                Value::Ref(id) => self.store.resolve(*id),
                _ => Err(invalid("candidate is not an entity IRI")),
            })
            .collect()
    }
    fn resolve(&self, term: &str) -> Result<String> {
        let term = term
            .strip_prefix('<')
            .and_then(|t| t.strip_suffix('>'))
            .unwrap_or(term);
        let expanded = crate::compact::PrefixMap::from_store(self.store)?.expand(term);
        if expanded.starts_with("http://")
            || expanded.starts_with("https://")
            || expanded.starts_with("urn:")
        {
            iri(&expanded)?;
            return Ok(expanded);
        }
        if term.contains(':') {
            return Err(invalid(format!(
                "unknown prefix in {term:?}; use a full IRI"
            )));
        }
        let pattern = format!("?s <{LABEL}> {} .", literal(term));
        let found = self.select(&pattern, "s")?;
        if found.len() != 1 {
            return Err(invalid(format!(
                "label {term:?} resolves to {} entities; use one full IRI",
                found.len()
            )));
        }
        Ok(found.into_iter().next().expect("one entity"))
    }
    fn candidates(&self, pattern: &str, seed: Option<&HashSet<String>>) -> Result<HashSet<String>> {
        if let Some(seed) = seed {
            if seed.is_empty() {
                return Ok(HashSet::new());
            }
            let mut values = seed.iter().map(|s| iri(s)).collect::<Result<Vec<_>>>()?;
            values.sort();
            self.select(
                &format!("VALUES ?s {{ {} }} {pattern}", values.join(" ")),
                "s",
            )
        } else {
            self.select(pattern, "s")
        }
    }
    fn field(&self, atom: &str, seed: Option<&HashSet<String>>) -> Result<HashSet<String>> {
        let operator = field_operator(self.store, atom)?;
        let Some((pos, op)) = operator else {
            return self.text(atom, seed);
        };
        let (field, value) = (&atom[..pos], &atom[pos + op.len()..]);
        if field.is_empty() || value.is_empty() {
            return Err(invalid(format!("missing field/value in {atom:?}")));
        }
        let predicate = if field == "type" {
            crate::namespace::RDF_TYPE.to_owned()
        } else {
            self.resolve(field)?
        };
        let quoted = value.starts_with('"');
        let value = if quoted {
            serde_json::from_str::<String>(value)
                .map_err(|e| invalid(format!("bad value in {atom:?}: {e}")))?
        } else {
            value.to_owned()
        };
        let predicate = iri(&predicate)?;
        let pattern = if op == ":" || op == "=" {
            if field == "type" || (!quoted && (value.starts_with('<') || value.contains(':'))) {
                format!("?s {predicate} {} .", iri(&self.resolve(&value)?)?)
            } else if value.ends_with('*') {
                let prefix = value.strip_suffix('*').expect("suffix checked");
                if prefix.is_empty() {
                    return Err(invalid("empty prefix is unbounded"));
                }
                format!(
                    "?s {predicate} ?v . FILTER(isLiteral(?v) && STRSTARTS(STR(?v), {}))",
                    literal(prefix)
                )
            } else {
                // Literal equality or an exact object label (one hop). Resolve
                // labels independently; multiple matching object labels refuse.
                let mut result = self.candidates(
                    &format!(
                        "?s {predicate} ?v . FILTER(isLiteral(?v) && STR(?v) = {})",
                        literal(&value)
                    ),
                    seed,
                )?;
                let labels = self.select(&format!("?s <{LABEL}> {} .", literal(&value)), "s")?;
                if labels.len() > 1 {
                    return Err(invalid(format!(
                        "object label {value:?} is ambiguous; use a full IRI"
                    )));
                }
                if let Some(object) = labels.iter().next() {
                    result.extend(
                        self.candidates(&format!("?s {predicate} {} .", iri(object)?), seed)?,
                    );
                }
                return capped(result);
            }
        } else if value.parse::<f64>().is_ok_and(f64::is_finite) {
            format!("?s {predicate} ?v . FILTER(?v {op} {value})")
        } else {
            // ISO date lexical order is meaningful only for validated, canonical
            // dates. Do not order arbitrary string attributes as numbers.
            if !canonical_date(&value) {
                return Err(invalid(format!(
                    "range {atom:?} requires a finite number or canonical YYYY-MM-DD date"
                )));
            }
            format!(
                "?s {predicate} ?v . FILTER(isLiteral(?v) && STR(?v) {op} {})",
                literal(&value)
            )
        };
        self.candidates(&pattern, seed)
    }
    fn text(&self, atom: &str, seed: Option<&HashSet<String>>) -> Result<HashSet<String>> {
        if !self.store.search_config().keyword
            || !self.store.lexical_progress()?.is_some_and(|p| p.complete)
        {
            return Err(invalid(
                "structured text needs an enabled, backfilled keyword index",
            ));
        }
        let (raw, suffix) = atom.strip_suffix('*').map_or((atom, ""), |s| (s, "*"));
        let raw = if raw.starts_with('"') {
            serde_json::from_str::<String>(raw)
                .map_err(|e| invalid(format!("bad text atom {atom:?}: {e}")))?
        } else {
            raw.to_owned()
        };
        if raw.is_empty() || raw.contains('*') {
            return Err(invalid(
                "text requires a nonempty term/phrase with at most one trailing wildcard",
            ));
        }
        let _progress = crate::time::request_deadline()
            .map(|d| sparql::ProgressGuard::install(&self.store.conn, d))
            .transpose()?;
        let expression = format!("\"{}\"{suffix}", raw.replace('"', "\"\""));
        let mut sql = String::from(
            "SELECT DISTINCT f.e FROM lexical_fts JOIN facts f ON f.rowid=lexical_fts.rowid WHERE lexical_fts MATCH ?1 AND f.g=0 AND f.op=1 AND ((?2 IS NULL AND f.valid_to IS NULL) OR (?2 IS NOT NULL AND f.valid_from<=?2 AND (f.valid_to IS NULL OR f.valid_to>?2)))",
        );
        let mut parameters = vec![
            rusqlite::types::Value::Text(expression),
            self.at.map_or(rusqlite::types::Value::Null, |s| {
                rusqlite::types::Value::Text(s.into())
            }),
        ];
        if let Some(seed) = seed {
            if seed.is_empty() {
                return Ok(HashSet::new());
            }
            let ids = seed
                .iter()
                .filter_map(|s| self.store.lookup(s).transpose())
                .collect::<Result<Vec<_>>>()?;
            if ids.is_empty() {
                return Ok(HashSet::new());
            }
            sql.push_str(&format!(" AND f.e IN ({})", vec!["?"; ids.len()].join(",")));
            parameters.extend(ids.into_iter().map(rusqlite::types::Value::Integer));
        }
        sql.push_str(" LIMIT 4097");
        let mut stmt = self.store.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(parameters), |row| {
            row.get::<_, i64>(0)
        })?;
        let mut found = HashSet::new();
        for row in rows {
            found.insert(self.store.resolve(row?)?);
        }
        capped(found)
    }
    pub(super) fn eval(
        &self,
        expr: &Expr,
        seed: Option<&HashSet<String>>,
    ) -> Result<HashSet<String>> {
        match expr {
            Expr::Atom(atom) => {
                let mut set = self.field(atom, seed)?;
                if let Some(seed) = seed {
                    set.retain(|s| seed.contains(s));
                }
                Ok(set)
            }
            Expr::Not(inner) => {
                let seed = seed
                    .ok_or_else(|| invalid("NOT requires a bounded positive candidate universe"))?;
                let excluded = self.eval(inner, Some(seed))?;
                Ok(seed.difference(&excluded).cloned().collect())
            }
            Expr::Or(left, right) => {
                let mut set = self.eval(left, seed)?;
                set.extend(self.eval(right, seed)?);
                capped(set)
            }
            Expr::And(left, right) => {
                // Evaluate the seeded arm first even when NOT was written first.
                let (first, second) = if structured_syntax::bounded(left, false) {
                    (left, right)
                } else {
                    (right, left)
                };
                let set = self.eval(first, seed)?;
                self.eval(second, Some(&set))
            }
        }
    }
}
fn field_operator<'a>(store: &Store, atom: &'a str) -> Result<Option<(usize, &'a str)>> {
    if atom.starts_with('"') {
        return Ok(None);
    }
    let start = if atom.starts_with('<') {
        atom.find('>')
            .ok_or_else(|| invalid("unclosed predicate IRI"))?
            + 1
    } else {
        0
    };
    let mut pos = start;
    if start == 0
        && let Some(colon) = atom.find(':')
    {
        let prefix = &atom[..colon + 1];
        if crate::compact::PrefixMap::from_store(store)?.expand(prefix) != prefix {
            pos = colon + 1;
        }
    }
    let found = [">=", "<=", ">", "<", "=", ":"]
        .iter()
        .filter_map(|op| atom[pos..].find(op).map(|n| (pos + n, *op)))
        .min_by_key(|(n, _)| *n);
    Ok(found.map(|(n, op)| (n, &atom[n..n + op.len()])))
}
fn capped(set: HashSet<String>) -> Result<HashSet<String>> {
    if set.len() > CAP {
        Err(invalid(
            "structured candidate cap 4096 exceeded; narrow the expression",
        ))
    } else {
        Ok(set)
    }
}
fn canonical_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10
        || !bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return false;
    }
    let year = s[..4].parse::<u32>().unwrap_or(0);
    let month = s[5..7].parse::<u32>().unwrap_or(0);
    let day = s[8..].parse::<u32>().unwrap_or(0);
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => 0,
    };
    year > 0 && day > 0 && day <= max
}
