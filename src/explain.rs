//! `explain` — walk a fact's derivation chain from the provenance already in
//! the fact log (gap G8 of `docs/design/semantic-reasoning-gaps.md`; phase 6
//! of `docs/design/reasoning-engine-fixes.md`, bead quipu-923).
//!
//! Every derived fact carries its deriver in the transaction `source`:
//! `reasoner:<rule-id>` for the Datalog engine, `owl:materialize` for the OWL
//! materializer. This module resolves that provenance into a tree — fact ←
//! rule/axiom ← premises — by RE-MATCHING the deriver's premises against the
//! current graph rather than reading stored justifications (support-set TMS
//! stays deferred; `docs/design/reasoner.md` Phase 5). A premise that has
//! since been retracted therefore shows as absent support, which is itself
//! diagnostic.
//!
//! ROOT-scoped: like `entity_facts`, the walk reads ROOT's own facts. Depth
//! is capped so mutually referring derivations terminate.

use rusqlite::params;
use serde_json::{Value as Json, json};

use crate::error::{Error, Result};
use crate::namespace::RDF_TYPE;
use crate::owl::{Axioms, Ontology, transitive_closure};
use crate::reasoner::ast::{BodyAtom, Rule, Term};
use crate::reasoner::{RuleSet, parse_rules};
use crate::store::Store;
use crate::types::Value;

/// Default recursion depth for the derivation walk.
pub const DEFAULT_EXPLAIN_DEPTH: usize = 5;

/// Explain one current fact `(subject, predicate, object)` — IRIs, with a
/// non-IRI object treated as a string literal. Returns a JSON tree.
pub fn explain(
    store: &Store,
    subject: &str,
    predicate: &str,
    object: &str,
    max_depth: usize,
) -> Result<Json> {
    let e = store
        .lookup(subject)?
        .ok_or_else(|| Error::InvalidValue(format!("unknown subject <{subject}>")))?;
    let a = store
        .lookup(predicate)?
        .ok_or_else(|| Error::InvalidValue(format!("unknown predicate <{predicate}>")))?;
    // An object that resolves to a term id is a reference; anything else can
    // only be stored as a literal.
    let value = match store.lookup(object)? {
        Some(id) => Value::Ref(id),
        None => Value::Str(object.to_string()),
    };

    let ctx = Context::load(store);
    explain_fact(store, &ctx, e, a, &value, max_depth)
}

/// The rulesets and axioms the walk resolves derivers against, loaded once.
struct Context {
    ruleset: Option<RuleSet>,
    axioms: Option<Axioms>,
}

impl Context {
    fn load(store: &Store) -> Self {
        let ruleset = store
            .get_combined_shapes()
            .ok()
            .flatten()
            .and_then(|ttl| parse_rules(&ttl, None).ok());
        let axioms = store
            .get_combined_ontologies()
            .ok()
            .flatten()
            .and_then(|ttl| Ontology::from_turtle(&ttl).ok())
            .map(|o| o.axioms);
        Self { ruleset, axioms }
    }
}

fn explain_fact(
    store: &Store,
    ctx: &Context,
    e: i64,
    a: i64,
    value: &Value,
    depth: usize,
) -> Result<Json> {
    let display = json!({
        "s": store.resolve(e)?,
        "p": store.resolve(a)?,
        "o": display_value(store, value)?,
    });
    let Some((tx, source, valid_from)) = fact_row(store, e, a, value)? else {
        return Ok(json!({ "fact": display, "found": false }));
    };
    let mut node = json!({
        "fact": display,
        "found": true,
        "tx": tx,
        "source": source,
        "valid_from": valid_from,
    });

    let derivation = match source.as_deref() {
        _ if depth == 0 => Some(json!({ "kind": "depth-capped" })),
        Some(src) if src.starts_with("reasoner:") => Some(explain_rule(
            store,
            ctx,
            src.trim_start_matches("reasoner:"),
            e,
            value,
            depth,
        )?),
        Some("owl:materialize") => Some(explain_owl(store, ctx, e, a, value, depth)?),
        // Plain sources (episode ingest, knot, tests, …) are base facts: the
        // transaction row IS the explanation.
        _ => None,
    };
    if let Some(d) = derivation {
        node["derivation"] = d;
    }
    Ok(node)
}

// ── Datalog rules ────────────────────────────────────────────────────

fn explain_rule(
    store: &Store,
    ctx: &Context,
    rule_id: &str,
    e: i64,
    value: &Value,
    depth: usize,
) -> Result<Json> {
    let Some(rule) = ctx
        .ruleset
        .as_ref()
        .and_then(|rs| rs.rules.iter().find(|r| r.id == rule_id))
    else {
        return Ok(json!({
            "kind": "rule",
            "rule": rule_id,
            "note": "rule not found in the stored shapes — it may have been \
                     removed since this fact was derived",
        }));
    };
    let Value::Ref(v_ref) = value else {
        return Ok(json!({
            "kind": "rule",
            "rule": rule_id,
            "note": "rule heads emit reference values; a literal here cannot \
                     have been this rule's output",
        }));
    };

    let premises = match rule_support(store, rule, e, *v_ref)? {
        Some(support) => {
            let mut out = Vec::new();
            for (pe, pred, pv) in support {
                let pa = store
                    .lookup(&pred)?
                    .ok_or_else(|| Error::InvalidValue(format!("unknown predicate <{pred}>")))?;
                out.push(explain_fact(
                    store,
                    ctx,
                    pe,
                    pa,
                    &Value::Ref(pv),
                    depth - 1,
                )?);
            }
            json!(out)
        }
        None => json!({
            "note": "no current premises re-match this derivation — its \
                     support may have been retracted since (re-run the \
                     reasoner to converge)",
        }),
    };

    Ok(json!({
        "kind": "rule",
        "rule": rule_id,
        "head": rule.head.predicate,
        "premises": premises,
    }))
}

/// Find one set of body facts that derives head tuple `(e, v)` for `rule`.
/// Returns `(entity, predicate-IRI, value-ref)` per positive body atom.
fn rule_support(
    store: &Store,
    rule: &Rule,
    e: i64,
    v: i64,
) -> Result<Option<Vec<(i64, String, i64)>>> {
    let body: Vec<_> = rule
        .body
        .iter()
        .filter_map(|b| match b {
            BodyAtom::Positive(atom) => Some(atom),
            BodyAtom::Negative(_) => None,
        })
        .collect();

    // Bind head variables to the derived tuple.
    let mut bindings: Vec<(String, i64)> = Vec::new();
    for (term, val) in rule.head.args.iter().zip([e, v]) {
        match term {
            Term::Var(name) => bindings.push((name.clone(), val)),
            Term::Iri(iri) => {
                if store.lookup(iri)? != Some(val) {
                    return Ok(None);
                }
            }
            Term::Str(_) => return Ok(None),
        }
    }

    match body.as_slice() {
        [atom] => Ok(match_atom(store, atom, &bindings)?
            .map(|(pe, pv, _)| vec![(pe, atom.predicate.clone(), pv)])),
        [left, right] => {
            let Some(left_facts) = atom_candidates(store, left, &bindings)? else {
                return Ok(None);
            };
            for (le, lv, l_bind) in left_facts {
                if let Some((re, rv, _)) = match_atom(store, right, &l_bind)? {
                    return Ok(Some(vec![
                        (le, left.predicate.clone(), lv),
                        (re, right.predicate.clone(), rv),
                    ]));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// One candidate body match: `(entity, ref-object, extended bindings)`.
type Candidate = (i64, i64, Vec<(String, i64)>);

/// First fact of `atom`'s predicate consistent with `bindings`, plus the
/// bindings extended by the match.
fn match_atom(
    store: &Store,
    atom: &crate::reasoner::Atom,
    bindings: &[(String, i64)],
) -> Result<Option<Candidate>> {
    Ok(atom_candidates(store, atom, bindings)?.and_then(|mut all| {
        if all.is_empty() {
            None
        } else {
            Some(all.remove(0))
        }
    }))
}

/// All facts of `atom`'s predicate consistent with `bindings`.
fn atom_candidates(
    store: &Store,
    atom: &crate::reasoner::Atom,
    bindings: &[(String, i64)],
) -> Result<Option<Vec<Candidate>>> {
    let Some(pred_id) = store.lookup(&atom.predicate)? else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for (fe, fv) in ref_pairs_for(store, pred_id)? {
        let mut extended = bindings.to_vec();
        let mut ok = true;
        for (term, val) in atom.args.iter().zip([fe, fv]) {
            match term {
                Term::Var(name) => match extended.iter().find(|(n, _)| n == name) {
                    Some((_, bound)) if *bound != val => {
                        ok = false;
                        break;
                    }
                    Some(_) => {}
                    None => extended.push((name.clone(), val)),
                },
                Term::Iri(iri) => {
                    if store.lookup(iri)? != Some(val) {
                        ok = false;
                        break;
                    }
                }
                Term::Str(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            out.push((fe, fv, extended));
        }
    }
    Ok(Some(out))
}

// ── OWL materialization ──────────────────────────────────────────────

/// Which axiom families could have produced this fact, each with the premise
/// facts that currently support it. More than one family can apply — all are
/// reported, because the fact log does not record which one fired.
fn explain_owl(
    store: &Store,
    ctx: &Context,
    e: i64,
    a: i64,
    value: &Value,
    depth: usize,
) -> Result<Json> {
    let Some(axioms) = ctx.axioms.as_ref() else {
        return Ok(json!({
            "kind": "owl",
            "note": "no ontologies stored — cannot resolve which axiom \
                     derived this fact",
        }));
    };
    let Value::Ref(v_ref) = value else {
        return Ok(json!({
            "kind": "owl",
            "note": "OWL materialization emits reference values only",
        }));
    };

    let p_iri = store.resolve(a)?;
    let o_iri = store.resolve(*v_ref)?;
    let mut families: Vec<Json> = Vec::new();
    let premise = |family: &str, axiom: String, facts: Vec<(i64, i64, i64)>| -> Result<Json> {
        let mut premises = Vec::new();
        for (pe, pa, pv) in facts {
            premises.push(explain_fact(
                store,
                ctx,
                pe,
                pa,
                &Value::Ref(pv),
                depth - 1,
            )?);
        }
        Ok(json!({ "family": family, "axiom": axiom, "premises": premises }))
    };

    if p_iri == RDF_TYPE {
        // Subclass closure: (e a C) with C ⊑ o.
        let closure = transitive_closure(&axioms.subclass_of);
        for class_id in refs_for(store, e, a)? {
            let class_iri = store.resolve(class_id)?;
            if class_iri != o_iri && closure.get(&class_iri).is_some_and(|s| s.contains(&o_iri)) {
                families.push(premise(
                    "subClassOf",
                    format!("{class_iri} rdfs:subClassOf {o_iri}"),
                    vec![(e, a, class_id)],
                )?);
            }
        }
        // Equivalent classes.
        for (x, y) in &axioms.equivalent_classes {
            let other = if *x == o_iri {
                y
            } else if *y == o_iri {
                x
            } else {
                continue;
            };
            if let Some(other_id) = store.lookup(other)?
                && exists_ref(store, e, a, other_id)?
            {
                families.push(premise(
                    "equivalentClass",
                    format!("{x} owl:equivalentClass {y}"),
                    vec![(e, a, other_id)],
                )?);
            }
        }
        // Domain / range.
        for (prop, class) in &axioms.domains {
            if class == &o_iri
                && let Some(prop_id) = store.lookup(prop)?
                && let Some(obj) = refs_for(store, e, prop_id)?.first()
            {
                families.push(premise(
                    "domain",
                    format!("{prop} rdfs:domain {class}"),
                    vec![(e, prop_id, *obj)],
                )?);
            }
        }
        for (prop, class) in &axioms.ranges {
            if class == &o_iri
                && let Some(prop_id) = store.lookup(prop)?
                && let Some(subj) = subjects_for(store, prop_id, e)?.first()
            {
                families.push(premise(
                    "range",
                    format!("{prop} rdfs:range {class}"),
                    vec![(*subj, prop_id, e)],
                )?);
            }
        }
    } else {
        // Subproperty / equivalentProperty: (e q o) restated under p.
        let closure = transitive_closure(&axioms.subproperty_of);
        for (sub, supers) in &closure {
            if supers.contains(&p_iri)
                && let Some(sub_id) = store.lookup(sub)?
                && exists_ref(store, e, sub_id, *v_ref)?
            {
                families.push(premise(
                    "subPropertyOf",
                    format!("{sub} rdfs:subPropertyOf {p_iri}"),
                    vec![(e, sub_id, *v_ref)],
                )?);
            }
        }
        for (x, y) in &axioms.equivalent_properties {
            let other = if *x == p_iri {
                y
            } else if *y == p_iri {
                x
            } else {
                continue;
            };
            if let Some(other_id) = store.lookup(other)?
                && exists_ref(store, e, other_id, *v_ref)?
            {
                families.push(premise(
                    "equivalentProperty",
                    format!("{x} owl:equivalentProperty {y}"),
                    vec![(e, other_id, *v_ref)],
                )?);
            }
        }
        // Inverse: (o q e) with q inverseOf p.
        for (q, p) in &axioms.inverse_of {
            if p == &p_iri
                && let Some(q_id) = store.lookup(q)?
                && exists_ref(store, *v_ref, q_id, e)?
            {
                families.push(premise(
                    "inverseOf",
                    format!("{q} owl:inverseOf {p}"),
                    vec![(*v_ref, q_id, e)],
                )?);
            }
        }
        // Symmetric: (o p e).
        if axioms.symmetric_properties.contains(&p_iri) && exists_ref(store, *v_ref, a, e)? {
            families.push(premise(
                "symmetric",
                format!("{p_iri} a owl:SymmetricProperty"),
                vec![(*v_ref, a, e)],
            )?);
        }
        // Transitive: one chain (e p m), (m p o).
        if axioms.transitive_properties.contains(&p_iri) {
            for m in refs_for(store, e, a)? {
                if m != *v_ref && exists_ref(store, m, a, *v_ref)? {
                    families.push(premise(
                        "transitive",
                        format!("{p_iri} a owl:TransitiveProperty"),
                        vec![(e, a, m), (m, a, *v_ref)],
                    )?);
                    break;
                }
            }
        }
    }

    if families.is_empty() {
        return Ok(json!({
            "kind": "owl",
            "note": "no loaded axiom currently re-derives this fact — the \
                     ontology or its premises may have changed since \
                     materialization",
        }));
    }
    Ok(json!({ "kind": "owl", "families": families }))
}

// ── Fact-log reads (ROOT-scoped) ─────────────────────────────────────

fn fact_row(
    store: &Store,
    e: i64,
    a: i64,
    value: &Value,
) -> Result<Option<(i64, Option<String>, String)>> {
    let mut stmt = store.conn.prepare(
        "SELECT f.tx, t.source, f.valid_from FROM facts f \
         JOIN transactions t ON f.tx = t.id \
         WHERE f.e = ?1 AND f.a = ?2 AND f.v = ?3 \
           AND f.op = 1 AND f.valid_to IS NULL AND f.g = 0 \
         ORDER BY f.tx DESC LIMIT 1",
    )?;
    let mut rows = stmt.query(params![e, a, value.to_bytes()])?;
    match rows.next()? {
        Some(row) => Ok(Some((row.get(0)?, row.get(1)?, row.get(2)?))),
        None => Ok(None),
    }
}

/// Current reference objects of `(e, a, ?o)`.
fn refs_for(store: &Store, e: i64, a: i64) -> Result<Vec<i64>> {
    let mut stmt = store.conn.prepare(
        "SELECT v FROM facts WHERE e = ?1 AND a = ?2 \
         AND op = 1 AND valid_to IS NULL AND g = 0",
    )?;
    collect_refs(stmt.query(params![e, a])?)
}

/// Current subjects of `(?s, a, o)`.
fn subjects_for(store: &Store, a: i64, o: i64) -> Result<Vec<i64>> {
    let o_bytes = Value::Ref(o).to_bytes();
    let mut stmt = store.conn.prepare(
        "SELECT e FROM facts WHERE a = ?1 AND v = ?2 \
         AND op = 1 AND valid_to IS NULL AND g = 0",
    )?;
    let mut rows = stmt.query(params![a, o_bytes])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(row.get(0)?);
    }
    Ok(out)
}

fn exists_ref(store: &Store, e: i64, a: i64, o: i64) -> Result<bool> {
    Ok(fact_row(store, e, a, &Value::Ref(o))?.is_some())
}

/// Current `(entity, ref-object)` pairs for a predicate.
fn ref_pairs_for(store: &Store, a: i64) -> Result<Vec<(i64, i64)>> {
    let mut stmt = store.conn.prepare(
        "SELECT e, v FROM facts WHERE a = ?1 \
         AND op = 1 AND valid_to IS NULL AND g = 0",
    )?;
    let mut rows = stmt.query(params![a])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let e: i64 = row.get(0)?;
        if let Value::Ref(o) = Value::from_bytes(&row.get::<_, Vec<u8>>(1)?)? {
            out.push((e, o));
        }
    }
    Ok(out)
}

fn collect_refs(mut rows: rusqlite::Rows<'_>) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        if let Value::Ref(o) = Value::from_bytes(&row.get::<_, Vec<u8>>(0)?)? {
            out.push(o);
        }
    }
    Ok(out)
}

fn display_value(store: &Store, value: &Value) -> Result<Json> {
    Ok(match value {
        Value::Ref(id) => json!(store.resolve(*id)?),
        other => json!(format!("{other:?}")),
    })
}

#[cfg(test)]
#[path = "explain_tests.rs"]
mod tests;
