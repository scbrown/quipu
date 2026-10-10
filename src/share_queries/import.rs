//! The receiving half of `queries.ttl`: seal check, vetting, namespacing and
//! installation (aegis-fxpbys.2). See the parent module for the contract.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{QUERIES_FILE, SharedQuery, from_turtle};
use crate::error::{Error, Result};
use crate::store::Store;

/// A same-name query already in the receiver's registry, left untouched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryCollision {
    /// The name inside the share.
    pub name: String,
    /// The pack-scoped name it would have been installed under.
    pub local: String,
    /// Why it was not installed.
    pub reason: String,
}

/// A query held back because the receiver cannot govern what it targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryQuarantine {
    /// The name inside the share.
    pub name: String,
    /// Targeted classes the receiver's loaded shapes do not sanction. Empty
    /// when the query itself is fine and the PACK is quarantined.
    pub off_vocabulary: Vec<String>,
}

/// What an import did with a share's `queries.ttl`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryImport {
    /// Registry namespace: every installed name is `<namespace>/<name>`.
    pub namespace: String,
    /// Newly installed pack-scoped names.
    pub installed: Vec<String>,
    /// Already present with an identical definition.
    pub unchanged: Vec<String>,
    /// Changed definitions replaced because the import asked to replace.
    pub replaced: Vec<String>,
    /// Pack-scoped names absent from this share, closed because the import
    /// asked to replace (never deleted — the prior version stays queryable).
    pub removed: Vec<String>,
    /// Pack-scoped names absent from this share and left in place.
    pub stale: Vec<String>,
    /// Same-name local definitions that differ; nothing was overwritten.
    pub collisions: Vec<QueryCollision>,
    /// Queries withheld from the registry with the quarantined pack.
    pub quarantined: Vec<QueryQuarantine>,
    /// Pack-scoped names held with a staged import; `import promote` installs
    /// them (aegis-9ofqqs). Nothing under these names is registered yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub awaiting_promotion: Vec<String>,
}

/// The default registry namespace for a share: stable across a producer's
/// versions (it keys on the producer's store identity, not the share id), so a
/// newer share of the same pack lands on the same names.
#[must_use]
pub fn default_namespace(store_id: &str) -> String {
    let digest = crate::share::sha256(store_id.as_bytes());
    format!("pack-{}", &digest[7..19])
}

/// Refuse a namespace that could escape its `<namespace>/` prefix.
///
/// # Errors
/// Empty, or holding anything but ASCII alphanumerics, `-`, `_` and `.`.
pub fn check_namespace(namespace: &str) -> Result<()> {
    if namespace.is_empty()
        || !namespace
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(Error::InvalidValue(format!(
            "query namespace {namespace:?} must be non-empty [A-Za-z0-9._-]"
        )));
    }
    Ok(())
}

/// Class IRIs a query targets that `sanctioned` does not contain.
#[must_use]
pub fn off_vocabulary(query: &SharedQuery, sanctioned: &BTreeSet<String>) -> Vec<String> {
    query
        .targets
        .iter()
        .filter(|t| !sanctioned.contains(*t))
        .cloned()
        .collect()
}

/// Install verified queries under `namespace`, never overwriting a differing
/// definition unless `replace` says so.
///
/// Evaluates nothing: every definition was parsed and validated by
/// [`from_turtle`] before this runs, and `query_load` only writes rows.
///
/// # Errors
/// Store errors.
pub fn install(
    store: &Store,
    queries: &[SharedQuery],
    namespace: &str,
    replace: bool,
    timestamp: &str,
) -> Result<QueryImport> {
    let mut report = QueryImport {
        namespace: namespace.to_string(),
        ..QueryImport::default()
    };
    let prefix = format!("{namespace}/");
    let mut carried = BTreeSet::new();
    for shared in queries {
        let mut local = shared.query.clone();
        local.name = format!("{prefix}{}", shared.query.name);
        carried.insert(local.name.clone());
        match store.query_get(&local.name)? {
            None => {
                store.query_load(&local, timestamp)?;
                report.installed.push(local.name);
            }
            Some(existing) if existing == local => report.unchanged.push(local.name),
            Some(_) if replace => {
                store.query_load(&local, timestamp)?;
                report.replaced.push(local.name);
            }
            Some(_) => report.collisions.push(QueryCollision {
                name: shared.query.name.clone(),
                local: local.name,
                reason: "a different definition is already registered under this name; \
                         re-import with replace to take the pack's version"
                    .into(),
            }),
        }
    }
    for existing in store.query_list()? {
        if existing.name.starts_with(&prefix) && !carried.contains(&existing.name) {
            if replace {
                store.query_remove(&existing.name, timestamp)?;
                report.removed.push(existing.name);
            } else {
                report.stale.push(existing.name);
            }
        }
    }
    Ok(report)
}

/// Check the member against the manifest that seals it.
///
/// # Errors
/// A declared member is missing or does not hash to its seal, or a member is
/// present that the manifest does not declare.
pub fn verify_member(manifest: &crate::share::ShareManifest, member: Option<&str>) -> Result<()> {
    match (
        manifest.queries_hash.as_deref(),
        manifest.files.queries.as_deref(),
        member,
    ) {
        (None, None, None) => Ok(()),
        (Some(hash), Some(QUERIES_FILE), Some(text)) => {
            let actual = crate::share::sha256(text.as_bytes());
            if actual == hash {
                Ok(())
            } else {
                Err(Error::InvalidValue(format!(
                    "share queries hash mismatch: manifest={hash} actual={actual}"
                )))
            }
        }
        (None, None, Some(_)) => Err(Error::InvalidValue(
            "share carries queries.ttl but its manifest does not declare it".into(),
        )),
        _ => Err(Error::InvalidValue(
            "share manifest declares queries.ttl but the member is missing or misnamed".into(),
        )),
    }
}

/// A share's queries, parsed and vetted before anything is staged.
#[derive(Debug)]
pub struct Pending {
    pub(super) queries: Vec<SharedQuery>,
    held: Vec<QueryQuarantine>,
    pub(super) namespace: String,
    pub(super) replace: bool,
    member: String,
    store_id: String,
}

impl Pending {
    /// Whether any query targets a class the receiver does not sanction.
    #[must_use]
    pub fn off_vocabulary(&self) -> bool {
        !self.held.is_empty()
    }
}

/// Parse the member and measure it against the receiver's vocabulary.
///
/// Runs BEFORE staging, so a refused member (an Update, a stray triple, a
/// shape violation) leaves nothing behind.
///
/// # Errors
/// See [`from_turtle`] and [`check_namespace`].
pub fn prepare(
    store: &Store,
    member: Option<&str>,
    store_id: &str,
    namespace: Option<&str>,
    replace: bool,
) -> Result<Option<Pending>> {
    let Some(member) = member else {
        return Ok(None);
    };
    let namespace = namespace.map_or_else(|| default_namespace(store_id), String::from);
    check_namespace(&namespace)?;
    let queries = from_turtle(member)?;
    let sanctioned = crate::vocabulary::sanctioned(store)?;
    let held = queries
        .iter()
        .filter_map(|q| {
            let off = off_vocabulary(q, &sanctioned);
            (!off.is_empty()).then(|| QueryQuarantine {
                name: q.query.name.clone(),
                off_vocabulary: off,
            })
        })
        .collect();
    Ok(Some(Pending {
        queries,
        held,
        namespace,
        replace,
        member: member.to_string(),
        store_id: store_id.to_string(),
    }))
}

/// Every query of `pending`, reported as withheld: the off-vocabulary ones
/// with the classes they target, the rest with none.
pub(super) fn withheld(pending: Pending) -> QueryImport {
    let mut held = pending.held;
    for q in &pending.queries {
        if !held.iter().any(|h| h.name == q.query.name) {
            held.push(QueryQuarantine {
                name: q.query.name.clone(),
                off_vocabulary: Vec::new(),
            });
        }
    }
    held.sort_by(|a, b| a.name.cmp(&b.name));
    QueryImport {
        namespace: pending.namespace,
        quarantined: held,
        ..QueryImport::default()
    }
}

/// Settle a prepared member at import. INSTALLS NOTHING (aegis-9ofqqs).
///
/// A quarantined pack's queries are reported as withheld and not kept: such a
/// pack cannot be promoted, and a fixed pack is re-imported. A staged pack's
/// queries are held under its share id and installed by `import promote`
/// ([`super::pending::release`]), behind the same human gate as its data.
///
/// # Errors
/// Store errors.
pub fn settle(
    store: &Store,
    share_id: &str,
    pending: Option<Pending>,
    quarantined: bool,
    timestamp: &str,
) -> Result<Option<QueryImport>> {
    let Some(pending) = pending else {
        return Ok(None);
    };
    if quarantined {
        return Ok(Some(withheld(pending)));
    }
    super::pending::hold(
        store,
        share_id,
        &pending.member,
        &pending.store_id,
        &pending.namespace,
        pending.replace,
        timestamp,
    )?;
    let mut awaiting: Vec<String> = pending
        .queries
        .iter()
        .map(|q| format!("{}/{}", pending.namespace, q.query.name))
        .collect();
    awaiting.sort();
    Ok(Some(QueryImport {
        namespace: pending.namespace,
        awaiting_promotion: awaiting,
        ..QueryImport::default()
    }))
}
