//! Opt-in, derived FTS5 index of ROOT assertions. Fact rowids are document ids;
//! triggers maintain it in the fact writer's transaction, including rollback.
//! Valid-time metadata is read from facts, so closing a fact does not rewrite
//! its text or destroy historical search. No search performs a backfill.

use rusqlite::{Connection, OptionalExtension, functions::FunctionFlags, params};

use crate::{Error, Result, Store, Value, vector::VectorMatch};

const COLUMNS: &str = "label, alt_label, description, attributes, type_names, iri_tokens, entity_iri, type_iri, language, datatype, graph_id";
// The source view is accessed by fact rowid (one trigger row or a bounded
// backfill range). NOT INDEXED preserves INTEGER PRIMARY KEY lookup: otherwise
// SQLite can choose the graph index and scan all ROOT facts for every batch.
const SCHEMA: &str = r#"
CREATE VIRTUAL TABLE IF NOT EXISTS lexical_fts USING fts5(
    label, alt_label, description, attributes, type_names, iri_tokens,
    entity_iri UNINDEXED, type_iri UNINDEXED, language UNINDEXED,
    datatype UNINDEXED, graph_id UNINDEXED,
    tokenize = 'unicode61'
);
CREATE TABLE IF NOT EXISTS lexical_progress (
    id INTEGER PRIMARY KEY CHECK(id=1), cursor INTEGER NOT NULL,
    highwater INTEGER NOT NULL, complete INTEGER NOT NULL,
    documents INTEGER NOT NULL DEFAULT 0 CHECK(documents>=0)
);
INSERT OR IGNORE INTO lexical_progress
    SELECT 1, 0, highwater, highwater=0, 0
    FROM (SELECT coalesce(max(rowid),0) AS highwater FROM facts);
CREATE VIEW IF NOT EXISTS lexical_source AS
SELECT f.rowid AS fact_id,
    CASE WHEN p.iri IN ('http://www.w3.org/2000/01/rdf-schema#label',
                       'http://www.w3.org/2004/02/skos/core#prefLabel')
         THEN quipu_lexical_text(f.v) ELSE '' END AS label,
    CASE WHEN p.iri='http://www.w3.org/2004/02/skos/core#altLabel'
         THEN quipu_lexical_text(f.v) ELSE '' END AS alt_label,
    CASE WHEN p.iri IN ('http://www.w3.org/2000/01/rdf-schema#comment',
                       'http://www.w3.org/2004/02/skos/core#definition')
         THEN quipu_lexical_text(f.v) ELSE '' END AS description,
    CASE WHEN p.iri NOT IN ('http://www.w3.org/2000/01/rdf-schema#label',
              'http://www.w3.org/2004/02/skos/core#prefLabel',
              'http://www.w3.org/2004/02/skos/core#altLabel',
              'http://www.w3.org/2000/01/rdf-schema#comment',
              'http://www.w3.org/2004/02/skos/core#definition')
         THEN quipu_lexical_text(f.v) ELSE '' END AS attributes,
    CASE WHEN p.iri='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
         THEN quipu_lexical_local(coalesce(o.iri,'')) ELSE '' END AS type_names,
    quipu_lexical_local(e.iri) AS iri_tokens,
    e.iri AS entity_iri,
    CASE WHEN p.iri='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
         THEN o.iri ELSE NULL END AS type_iri,
    quipu_lexical_language(f.v) AS language,
    quipu_lexical_datatype(f.v) AS datatype,
    f.g AS graph_id
FROM facts f NOT INDEXED JOIN terms e ON e.id=f.e JOIN terms p ON p.id=f.a
LEFT JOIN terms o ON o.id=quipu_lexical_ref(f.v)
WHERE f.op=1 AND f.g=0;
CREATE TRIGGER IF NOT EXISTS lexical_insert AFTER INSERT ON facts BEGIN
    UPDATE lexical_progress SET documents=documents+1 WHERE id=1
        AND NEW.op=1 AND NEW.g=0 AND NOT EXISTS(SELECT 1 FROM lexical_fts WHERE rowid=NEW.rowid);
    INSERT OR REPLACE INTO lexical_fts(rowid,label,alt_label,description,attributes,type_names,iri_tokens,entity_iri,type_iri,language,datatype,graph_id)
    SELECT fact_id,label,alt_label,description,attributes,type_names,iri_tokens,entity_iri,type_iri,language,datatype,graph_id
    FROM lexical_source WHERE fact_id=NEW.rowid;
END;
CREATE TRIGGER IF NOT EXISTS lexical_delete AFTER DELETE ON facts BEGIN
    UPDATE lexical_progress SET documents=documents-1 WHERE id=1
        AND EXISTS(SELECT 1 FROM lexical_fts WHERE rowid=OLD.rowid);
    DELETE FROM lexical_fts WHERE rowid=OLD.rowid;
END;
CREATE TRIGGER IF NOT EXISTS lexical_update AFTER UPDATE OF e,a,v,g,op ON facts BEGIN
    UPDATE lexical_progress SET documents=documents-1 WHERE id=1
        AND EXISTS(SELECT 1 FROM lexical_fts WHERE rowid=OLD.rowid);
    DELETE FROM lexical_fts WHERE rowid=OLD.rowid;
    UPDATE lexical_progress SET documents=documents+1 WHERE id=1 AND NEW.op=1 AND NEW.g=0;
    INSERT OR REPLACE INTO lexical_fts(rowid,label,alt_label,description,attributes,type_names,iri_tokens,entity_iri,type_iri,language,datatype,graph_id)
    SELECT fact_id,label,alt_label,description,attributes,type_names,iri_tokens,entity_iri,type_iri,language,datatype,graph_id
    FROM lexical_source WHERE fact_id=NEW.rowid;
END;
"#;

/// Register on every Store connection, including default-off writers reopening
/// an already-indexed database. A trigger must never depend on one caller's
/// decoder installation. Functions are deterministic and have no side effects.
pub(crate) fn register(conn: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    conn.create_scalar_function("quipu_lexical_language", 1, flags, |ctx| {
        let bytes: Vec<u8> = ctx.get(0)?;
        let value = Value::from_bytes(&bytes)
            .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
        Ok(value.language().map(str::to_owned))
    })?;
    conn.create_scalar_function("quipu_lexical_datatype", 1, flags, |ctx| {
        let bytes: Vec<u8> = ctx.get(0)?;
        let value = Value::from_bytes(&bytes)
            .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
        Ok(value.datatype().map(str::to_owned))
    })?;
    conn.create_scalar_function("quipu_lexical_text", 1, flags, |ctx| {
        let bytes: Vec<u8> = ctx.get(0)?;
        let value = Value::from_bytes(&bytes)
            .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
        Ok(match value {
            Value::Str(s) | Value::Lang { lexical: s, .. } | Value::Typed { lexical: s, .. } => s,
            Value::Int(n) => n.to_string(),
            Value::Float(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Ref(_) | Value::Bytes(_) => String::new(),
        })
    })?;
    conn.create_scalar_function("quipu_lexical_ref", 1, flags, |ctx| {
        let bytes: Vec<u8> = ctx.get(0)?;
        let value = Value::from_bytes(&bytes)
            .map_err(|e| rusqlite::Error::UserFunctionError(Box::new(e)))?;
        Ok(if let Value::Ref(id) = value {
            Some(id)
        } else {
            None
        })
    })?;
    conn.create_scalar_function("quipu_lexical_local", 1, flags, |ctx| {
        let iri: String = ctx.get(0)?;
        Ok(local_tokens(&iri))
    })
}

fn local_tokens(iri: &str) -> String {
    let local = iri.rsplit(['/', '#', ':']).next().unwrap_or(iri);
    // Keep the original spelling and add CamelCase word boundaries. FTS5's
    // unicode tokenizer supplies punctuation/hyphen/underscore boundaries.
    let mut words = String::new();
    let mut previous_lower = false;
    for c in local.chars() {
        if previous_lower && c.is_uppercase() {
            words.push(' ');
        }
        words.push(c);
        previous_lower = c.is_lowercase();
    }
    format!("{local} {words}")
}

/// Literal terms and double-quoted phrases, never caller-supplied FTS syntax.
/// Fielded/boolean expressions belong to the later structured-query stage.
fn match_expression(query: &str) -> Result<String> {
    if query.len() > 16_384 {
        return Err(Error::InvalidValue(
            "keyword query exceeds 16384 bytes".into(),
        ));
    }
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut quoted = false;
    for c in query.chars() {
        if c == '"' {
            if !part.trim().is_empty() {
                parts.push(std::mem::take(&mut part));
            }
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            if !part.is_empty() {
                parts.push(std::mem::take(&mut part));
            }
        } else {
            part.push(c);
        }
    }
    if quoted {
        return Err(Error::InvalidValue("unclosed keyword phrase".into()));
    }
    if !part.trim().is_empty() {
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(Error::InvalidValue("keyword query is empty".into()));
    }
    Ok(parts
        .into_iter()
        .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND "))
}

#[derive(Debug, serde::Serialize)]
pub struct LexicalProgress {
    pub cursor: i64,
    pub highwater: i64,
    pub complete: bool,
    pub documents: i64,
}

/// Metadata of the winning literal/type assertion, retained separately from
/// the text so language and typed literals never collapse into plain strings.
#[derive(Debug)]
pub struct LexicalMatch {
    pub matched: VectorMatch,
    pub language: Option<String>,
    pub datatype: Option<String>,
    pub type_iri: Option<String>,
}

impl Store {
    /// Create only the empty schema/triggers and capture a finite backfill
    /// highwater. Explicit activation, never a full-corpus startup migration.
    pub fn initialize_lexical_index(&self) -> Result<()> {
        // Do not re-run DDL or scan the corpus at the start of EVERY batch.
        // Successful setup is atomic, so its progress row proves it completed.
        if self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='lexical_progress')",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(());
        }
        register(&self.conn)?;
        self.conn.execute_batch("SAVEPOINT lexical_setup")?;
        let outcome = self.conn.execute_batch(SCHEMA);
        if outcome.is_err() {
            self.conn
                .execute_batch("ROLLBACK TO lexical_setup; RELEASE lexical_setup")?;
        } else {
            self.conn.execute_batch("RELEASE lexical_setup")?;
        }
        outcome?;
        Ok(())
    }

    pub fn lexical_progress(&self) -> Result<Option<LexicalProgress>> {
        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='lexical_progress')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        Ok(self
            .conn
            .query_row(
                "SELECT cursor,highwater,complete,documents FROM lexical_progress WHERE id=1",
                [],
                |r| {
                    Ok(LexicalProgress {
                        cursor: r.get(0)?,
                        highwater: r.get(1)?,
                        complete: r.get(2)?,
                        documents: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// Exactly one bounded transaction. The caller releases its Store/writer
    /// guard between calls. Durable cursor + insertion triggers make retries
    /// and concurrent writes safe. Never invoked implicitly by search.
    pub fn backfill_lexical_batch(&self, batch_size: usize) -> Result<LexicalProgress> {
        if !(1..=10_000).contains(&batch_size) {
            return Err(Error::InvalidValue(
                "lexical batch size must be 1..10000".into(),
            ));
        }
        self.initialize_lexical_index()?;
        self.conn.execute_batch("SAVEPOINT lexical_backfill")?;
        let outcome = (|| -> Result<()> {
            let (cursor, highwater): (i64, i64) = self.conn.query_row(
                "SELECT cursor,highwater FROM lexical_progress WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let end: i64 = self.conn.query_row(
                "SELECT coalesce(max(rowid),?2) FROM (SELECT rowid FROM facts WHERE rowid>?1 AND rowid<=?2 ORDER BY rowid LIMIT ?3)",
                params![cursor,highwater,batch_size as i64], |r| r.get(0))?;
            // Count only this bounded rowid range, never the whole growing
            // index on every batch (which would make backfill quadratic).
            let previous: i64 = self.conn.query_row(
                "SELECT count(*) FROM lexical_fts WHERE rowid>?1 AND rowid<=?2",
                params![cursor, end],
                |r| r.get(0),
            )?;
            let replacement: i64 = self.conn.query_row(
                "SELECT count(*) FROM lexical_source WHERE fact_id>?1 AND fact_id<=?2",
                params![cursor, end],
                |r| r.get(0),
            )?;
            self.conn.execute(&format!(
                "INSERT OR REPLACE INTO lexical_fts(rowid,{COLUMNS}) SELECT fact_id,{COLUMNS} FROM lexical_source WHERE fact_id>?1 AND fact_id<=?2"),
                params![cursor,end])?;
            self.conn.execute(
                "UPDATE lexical_progress SET cursor=?1, complete=(?1>=highwater), documents=documents+?2-?3 WHERE id=1",
                params![end,replacement,previous],
            )?;
            Ok(())
        })();
        if outcome.is_err() {
            self.conn
                .execute_batch("ROLLBACK TO lexical_backfill; RELEASE lexical_backfill")?;
        } else {
            self.conn.execute_batch("RELEASE lexical_backfill")?;
        }
        outcome?;
        self.lexical_progress()?
            .ok_or_else(|| Error::Store("lexical progress disappeared".into()))
    }

    /// Derived data only; removes triggers before dropping their target.
    pub fn drop_lexical_index(&self) -> Result<()> {
        self.conn.execute_batch("SAVEPOINT lexical_drop; DROP TRIGGER IF EXISTS lexical_insert; DROP TRIGGER IF EXISTS lexical_update; DROP TRIGGER IF EXISTS lexical_delete; DROP VIEW IF EXISTS lexical_source; DROP TABLE IF EXISTS lexical_fts; DROP TABLE IF EXISTS lexical_progress; RELEASE lexical_drop")?;
        Ok(())
    }

    pub fn keyword_search(
        &self,
        query: &str,
        limit: usize,
        valid_at: Option<&str>,
        allowed: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<VectorMatch>> {
        Ok(self
            .keyword_search_hits(query, limit, valid_at, allowed)?
            .into_iter()
            .map(|h| h.matched)
            .collect())
    }

    pub fn keyword_search_hits(
        &self,
        query: &str,
        limit: usize,
        valid_at: Option<&str>,
        allowed: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<LexicalMatch>> {
        let started = crate::time::Stopwatch::start();
        let deadline = crate::time::request_deadline().or_else(|| {
            let ms = self.search_config().query_timeout_ms;
            (ms > 0).then(|| crate::time::Deadline::after_millis(ms))
        });
        let _guard = deadline
            .map(|d| crate::sparql::ProgressGuard::install(&self.conn, d))
            .transpose()?;
        let result = self.keyword_search_inner(query, limit, valid_at, allowed);
        match result {
            Err(_) if deadline.is_some_and(|d| d.passed()) => Err(Error::QueryTimeout {
                elapsed_ms: started.elapsed_ms(),
                limit_ms: deadline
                    .map(|d| d.millis_from(&started))
                    .unwrap_or_default(),
            }),
            other => other,
        }
    }

    fn keyword_search_inner(
        &self,
        query: &str,
        limit: usize,
        valid_at: Option<&str>,
        allowed: Option<&std::collections::HashSet<String>>,
    ) -> Result<Vec<LexicalMatch>> {
        if !self.search_config().keyword {
            return Err(Error::InvalidValue(
                "keyword search is disabled ([quipu.search] keyword = false)".into(),
            ));
        }
        // Cheap state check: never count the index on a search request.
        let state: Option<bool> = if self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='lexical_progress')",
            [],
            |r| r.get::<_, bool>(0),
        )? {
            self.conn
                .query_row(
                    "SELECT complete FROM lexical_progress WHERE id=1",
                    [],
                    |r| r.get(0),
                )
                .optional()?
        } else {
            None
        };
        if state != Some(true) {
            return Err(Error::InvalidValue(
                "keyword index is not ready; run bounded lexical backfill batches".into(),
            ));
        }
        let expr = match_expression(query)?;
        let sql = "SELECT f.e, coalesce(nullif(label,''),nullif(alt_label,''),nullif(description,''),nullif(attributes,''),nullif(type_names,''),iri_tokens), -bm25(lexical_fts), f.valid_from, f.valid_to, language, datatype, type_iri
            FROM lexical_fts JOIN facts f ON f.rowid=lexical_fts.rowid
            WHERE lexical_fts MATCH ?1 AND f.g=0 AND f.op=1
            AND ((?2 IS NULL AND f.valid_to IS NULL) OR (?2 IS NOT NULL AND f.valid_from<=?2 AND (f.valid_to IS NULL OR f.valid_to>?2)))
            ORDER BY bm25(lexical_fts), f.e, f.rowid";
        let mut stmt = self.conn.prepare(sql)?;
        let mut rows = stmt.query(params![expr, valid_at])?;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let entity_id: i64 = r.get(0)?;
            if let Some(scope) = allowed
                && !scope.contains(&self.resolve(entity_id)?)
            {
                continue;
            }
            if !seen.insert(entity_id) {
                continue;
            }
            out.push(LexicalMatch {
                matched: VectorMatch {
                    entity_id,
                    text: r.get(1)?,
                    score: r.get(2)?,
                    valid_from: r.get(3)?,
                    valid_to: r.get(4)?,
                },
                language: r.get(5)?,
                datatype: r.get(6)?,
                type_iri: r.get(7)?,
            });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
