use rusqlite::params;

use super::{Evidence, Object, Request, Truth, Witness};
use crate::{Error, Result, Store, Value};

pub(super) fn read(store: &Store, request: &Request) -> Result<Evidence> {
    // Validate the instrument before any dictionary-miss shortcut. A missing
    // table/column must remain UNKNOWN even for an uninterned subject.
    store
        .conn
        .prepare("SELECT e, a, v, tx, valid_from, valid_to, op, g FROM facts LIMIT 0")?;
    let store_id = store.store_id()?;
    let transaction = store.latest_tx_id()?;
    let subject = store.lookup(&request.subject)?;
    let object = match &request.object {
        Object::Iri(iri) => store.lookup(iri)?.map(Value::Ref),
        Object::Str(value) => Some(Value::Str(value.clone())),
    };
    let positive = support(
        store,
        subject,
        store.lookup(&request.positive_predicate)?,
        object.as_ref(),
        request.witness_limit,
    )?;
    let negative = support(
        store,
        subject,
        store.lookup(&request.negative_predicate)?,
        object.as_ref(),
        request.witness_limit,
    )?;
    Ok(Evidence::Complete {
        truth: Truth::from_support(!positive.is_empty(), !negative.is_empty()),
        positive_supported: !positive.is_empty(),
        negative_supported: !negative.is_empty(),
        positive,
        negative,
        store_id,
        transaction,
        graph: crate::schema::ROOT_GRAPH_IRI.into(),
        scope: "current_primary_root_all_physical_assertions".into(),
    })
}

fn support(
    store: &Store,
    subject: Option<i64>,
    predicate: Option<i64>,
    object: Option<&Value>,
    limit: usize,
) -> Result<Vec<Witness>> {
    let (Some(subject), Some(predicate), Some(object)) = (subject, predicate, object) else {
        // Dictionary absence is observed within the same primary-store snapshot.
        return Ok(Vec::new());
    };
    let mut witnesses = Vec::new();
    let mut metadata_bytes = 0_i64;
    // Even xsd:string's historical tagged representation must participate.
    // Each query binds the exact indexed graph/entity/attribute/value tuple.
    for value in object.physical_aliases() {
        let mut statement = store.conn.prepare(
            "SELECT f.tx, length(CAST(f.valid_from AS BLOB)), \
             length(CAST(t.source AS BLOB)), f.valid_from, t.source FROM facts f \
             LEFT JOIN transactions t ON t.id = f.tx \
             WHERE f.g = 0 AND f.e = ?1 AND f.a = ?2 AND f.v = ?3 \
             AND f.op = 1 AND f.valid_to IS NULL \
             ORDER BY f.tx, f.valid_from LIMIT ?4",
        )?;
        // Request validation bounds this to 257; keep the SQL integer explicit.
        let sql_limit = i64::try_from(limit + 1)
            .map_err(|_| Error::InvalidValue("support witness cap overflow".into()))?;
        let mut rows = statement.query(params![subject, predicate, value, sql_limit])?;
        while let Some(row) = rows.next()? {
            let valid_from_bytes: i64 = row.get(1)?;
            let source_bytes: Option<i64> = row.get(2)?;
            let source_bytes = source_bytes.unwrap_or(0);
            if !(0..=16384).contains(&valid_from_bytes) || !(0..=16384).contains(&source_bytes) {
                return Err(Error::InvalidValue(
                    "support metadata field cap exceeded".into(),
                ));
            }
            metadata_bytes += valid_from_bytes + source_bytes;
            if metadata_bytes > 65536 {
                return Err(Error::InvalidValue(
                    "support metadata byte cap exceeded".into(),
                ));
            }
            witnesses.push(Witness {
                transaction: row.get(0)?,
                valid_from: row.get(3)?,
                source: row.get(4)?,
            });
            if witnesses.len() > limit {
                return Err(Error::InvalidValue("support witness cap exceeded".into()));
            }
        }
    }
    witnesses.sort_by(|left, right| {
        (&left.transaction, &left.valid_from, &left.source).cmp(&(
            &right.transaction,
            &right.valid_from,
            &right.source,
        ))
    });
    Ok(witnesses)
}
