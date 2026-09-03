//! Stored named-query registry (quipu #79).
//!
//! Design: `docs/design/knowledge-packs.md` §2. The compiled-in catalog
//! (`src/mcp/named_query.rs`) is a flat global array with no scoping field, so
//! a consumer cannot ship competency questions with its domain. This is the
//! storable half: same `NamedQuery` / `ParamSpec` shape, owned rather than
//! `&'static`, versioned in #71's close-don't-overwrite style.
//!
//! ## Validation happens at LOAD, not at call
//!
//! A stored query is a definition someone else will invoke, possibly much
//! later, possibly without reading it. Every way it can be malformed is
//! therefore rejected when it is written, while the author is present to fix
//! it:
//!
//! - the template must PARSE, with placeholders substituted;
//! - every `{placeholder}` must have a spec;
//! - an optional param must have a default.
//!
//! That last one closes a latent hole the compiled-in catalog still has: an
//! optional param with no default is simply `continue`d in `render`, leaving
//! `{param}` **verbatim in the SPARQL**. Compiled-in entries are reviewed, so
//! it has never bitten; a stored entry is not, so it is refused here.

use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::error::{Error, Result};

use super::Store;

/// A stored parameter spec — the owned twin of `mcp::named_query::ParamSpec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredParam {
    /// Placeholder name; appears in the template as `{name}`.
    pub name: String,
    /// `iri` | `text` | `int`.
    pub kind: String,
    /// Whether the caller must supply it.
    pub required: bool,
    /// Value used when an optional param is omitted.
    pub default: Option<String>,
    /// Human-readable description for the self-describing catalog.
    pub description: String,
}

/// A stored named query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredQuery {
    /// Stable identifier the caller passes as `name`.
    pub name: String,
    /// What the query answers.
    pub description: String,
    /// SPARQL template with `{param}` placeholders.
    pub template: String,
    /// Dataset scope (#69). `None` = global.
    pub dataset: Option<String>,
    /// Parameter specs, in display order.
    pub params: Vec<StoredParam>,
}

/// Every `{placeholder}` in `template`, in order of first appearance.
///
/// A placeholder is `{` + `[A-Za-z0-9_]+` + `}`. Everything else — most
/// importantly SPARQL's own group braces — is skipped.
///
/// ⚠️ **Advance by ONE on a non-match, never past the closing brace.** The
/// first `{` in `SELECT … WHERE { <{entity}> … }` is the WHERE group, whose
/// `}` is at the very end; skipping to it swallows the entire query body and
/// finds no placeholders at all. That is not a near-miss — it makes validation
/// vacuous on every realistic template, and it reported a correct query's
/// params as "declared but never used". Caught by the round-trip test.
fn placeholders(template: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{'
            && let Some(end) = template[i + 1..].find('}')
        {
            let name = &template[i + 1..i + 1 + end];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                if !out.iter().any(|n| n == name) {
                    out.push(name.to_string());
                }
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A stand-in value used only to make the template parseable during validation.
///
/// **Must mirror `ParamKind::render` exactly.** That renderer emits BARE
/// values — the *template* supplies the `<>` around an IRI and the `'…'` around
/// text (`<{entity}>`, `'{needle}'`). A probe that wrapped them would validate
/// `<<urn:probe>>`, i.e. a string production can never generate: the check
/// would reject good templates and, worse, could accept bad ones for the same
/// reason. Any change to `ParamKind::render` needs a matching change here.
fn probe_value(kind: &str) -> &'static str {
    match kind {
        "iri" => "urn:probe",
        "int" => "1",
        _ => "probe",
    }
}

impl StoredQuery {
    /// Reject every way this definition could be malformed (see module docs).
    ///
    /// # Errors
    /// Unknown kind, duplicate param, `{placeholder}` with no spec, spec with
    /// no placeholder, optional param with no default, or a template that does
    /// not parse once substituted.
    pub fn validate(&self) -> Result<()> {
        let mut seen: Vec<&str> = Vec::new();
        for p in &self.params {
            if !matches!(p.kind.as_str(), "iri" | "text" | "int") {
                return Err(Error::InvalidValue(format!(
                    "query '{}': param '{}' has kind '{}'; expected iri|text|int",
                    self.name, p.name, p.kind
                )));
            }
            if seen.contains(&p.name.as_str()) {
                return Err(Error::InvalidValue(format!(
                    "query '{}': param '{}' is declared twice",
                    self.name, p.name
                )));
            }
            seen.push(&p.name);
            // The latent hole, closed for stored entries: `render` skips an
            // omitted optional with no default, leaving `{param}` verbatim in
            // the SPARQL that then goes to the parser.
            if !p.required && p.default.is_none() {
                return Err(Error::InvalidValue(format!(
                    "query '{}': param '{}' is optional but has no default. An \
                     omitted optional with no default leaves '{{{}}}' verbatim in \
                     the rendered SPARQL — make it required, or give it a default.",
                    self.name, p.name, p.name
                )));
            }
        }

        let found = placeholders(&self.template);
        for ph in &found {
            if !self.params.iter().any(|p| &p.name == ph) {
                return Err(Error::InvalidValue(format!(
                    "query '{}': template references '{{{ph}}}' but no param spec \
                     declares it",
                    self.name
                )));
            }
        }
        for p in &self.params {
            if !found.iter().any(|ph| ph == &p.name) {
                return Err(Error::InvalidValue(format!(
                    "query '{}': param '{}' is declared but never used in the \
                     template",
                    self.name, p.name
                )));
            }
        }

        // Substitute probes and PARSE. A template that only parses for some
        // argument values is not a template, and finding that out at call time
        // means finding out in someone else's session.
        let mut probe = self.template.clone();
        for p in &self.params {
            probe = probe.replace(&format!("{{{}}}", p.name), probe_value(&p.kind));
        }
        crate::sparql::sparql_parser()
            .parse_query(&probe)
            .map_err(|e| {
                Error::InvalidValue(format!(
                    "query '{}': template does not parse as SPARQL: {e}",
                    self.name
                ))
            })?;
        Ok(())
    }
}

impl StoredQuery {
    /// Build executable SPARQL by validating and substituting `args`.
    ///
    /// Renders through `ParamKind::render` — the SAME function the compiled-in
    /// catalog uses — so a stored query and an identical builtin one produce
    /// byte-identical SPARQL. A second renderer would drift, and the drift
    /// would surface as two queries that look the same behaving differently.
    ///
    /// # Errors
    /// A missing required param, or a value the kind rejects.
    pub fn render(&self, args: &std::collections::BTreeMap<String, String>) -> Result<String> {
        let mut sparql = self.template.clone();
        for spec in &self.params {
            let raw = match args.get(&spec.name) {
                Some(v) => v.clone(),
                None => match &spec.default {
                    Some(d) => d.clone(),
                    // Unreachable for a STORED query — `validate` refuses an
                    // optional with no default at load. Kept as a refusal
                    // rather than a `continue`, because a `continue` here is
                    // exactly the hole that leaves `{param}` in the SPARQL.
                    None => {
                        return Err(Error::InvalidValue(format!(
                            "named query '{}' requires param '{}'",
                            self.name, spec.name
                        )));
                    }
                },
            };
            let kind =
                crate::mcp::named_query::ParamKind::from_label(&spec.kind).ok_or_else(|| {
                    Error::InvalidValue(format!(
                        "named query '{}': param '{}' has unknown kind '{}'",
                        self.name, spec.name, spec.kind
                    ))
                })?;
            let rendered = kind.render(&spec.name, &raw)?;
            sparql = sparql.replace(&format!("{{{}}}", spec.name), &rendered);
        }
        Ok(sparql)
    }

    /// The self-describing catalog entry, flagged `stored` so a caller can tell
    /// it from a compiled-in one.
    #[must_use]
    pub fn to_catalog_json(&self) -> serde_json::Value {
        let params: Vec<serde_json::Value> = self
            .params
            .iter()
            .map(|p| {
                serde_json::json!({
                    "name": p.name,
                    "type": p.kind,
                    "required": p.required,
                    "default": p.default,
                    "description": p.description,
                })
            })
            .collect();
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "params": params,
            "dataset": self.dataset,
            "source": "stored",
        })
    }
}

impl Store {
    /// Store a named query, **closing** any prior version (quipu #79).
    ///
    /// Validated before anything is written — see [`StoredQuery::validate`].
    ///
    /// # Errors
    /// Validation failures, and store errors.
    pub fn query_load(&self, q: &StoredQuery, timestamp: &str) -> Result<()> {
        q.validate()?;
        let tx = self.latest_tx_id()?;
        self.conn.execute_batch("SAVEPOINT quipu_query_load")?;
        let result = (|| -> Result<()> {
            self.conn.execute(
                "UPDATE queries SET valid_to = ?2 \
                 WHERE name = ?1 AND valid_to IS NULL AND valid_from < ?2",
                params![q.name, timestamp],
            )?;
            self.conn.execute(
                "DELETE FROM queries WHERE name = ?1 AND valid_from = ?2",
                params![q.name, timestamp],
            )?;
            self.conn.execute(
                "DELETE FROM query_params WHERE name = ?1 AND valid_from = ?2",
                params![q.name, timestamp],
            )?;
            self.conn.execute(
                "INSERT INTO queries (name, description, template, dataset, valid_from, valid_to, tx) \
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
                params![q.name, q.description, q.template, q.dataset, timestamp, tx],
            )?;
            for (ord, p) in q.params.iter().enumerate() {
                self.conn.execute(
                    "INSERT INTO query_params \
                     (name, valid_from, ord, param, kind, required, default_val, description) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        q.name,
                        timestamp,
                        i64::try_from(ord).unwrap_or(i64::MAX),
                        p.name,
                        p.kind,
                        i64::from(p.required),
                        p.default,
                        p.description,
                    ],
                )?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute_batch("RELEASE quipu_query_load")?;
                Ok(())
            }
            Err(e) => {
                let _ = self
                    .conn
                    .execute_batch("ROLLBACK TO quipu_query_load; RELEASE quipu_query_load");
                Err(e)
            }
        }
    }

    /// A stored query by name, current version only.
    pub fn query_get(&self, name: &str) -> Result<Option<StoredQuery>> {
        let row: Option<(String, String, String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT name, description, template, dataset FROM queries \
                 WHERE name = ?1 AND valid_to IS NULL",
                params![name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((name, description, template, dataset)) = row else {
            return Ok(None);
        };
        Ok(Some(StoredQuery {
            params: self.query_params_of(&name)?,
            name,
            description,
            template,
            dataset,
        }))
    }

    fn query_params_of(&self, name: &str) -> Result<Vec<StoredParam>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.param, p.kind, p.required, p.default_val, p.description \
             FROM query_params p JOIN queries q \
               ON q.name = p.name AND q.valid_from = p.valid_from \
             WHERE p.name = ?1 AND q.valid_to IS NULL ORDER BY p.ord",
        )?;
        let out = stmt
            .query_map(params![name], |r| {
                Ok(StoredParam {
                    name: r.get(0)?,
                    kind: r.get(1)?,
                    required: r.get::<_, i64>(2)? != 0,
                    default: r.get(3)?,
                    description: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(out)
    }

    /// Every current stored query, by name.
    pub fn query_list(&self) -> Result<Vec<StoredQuery>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM queries WHERE valid_to IS NULL ORDER BY name")?;
        let names = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(names.len());
        for n in names {
            if let Some(q) = self.query_get(&n)? {
                out.push(q);
            }
        }
        Ok(out)
    }

    /// Close a stored query's current version. **Never deletes** — the prior
    /// definition stays queryable, as #71 established for shapes.
    pub fn query_remove(&self, name: &str, timestamp: &str) -> Result<bool> {
        let affected = self.conn.execute(
            "UPDATE queries SET valid_to = ?2 WHERE name = ?1 AND valid_to IS NULL",
            params![name, timestamp],
        )?;
        Ok(affected > 0)
    }
}

#[cfg(test)]
#[path = "queries_tests.rs"]
mod tests;
