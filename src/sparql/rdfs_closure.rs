//! RDFS closure, MATERIALISED into a graph's companion inferred graph.
//!
//! ## Why this is not [`super::rdfs`], and not a reasoner ruleset
//!
//! Two mechanisms already look like they should serve RDFS entailment, and
//! neither can. Both fail for the SAME reason — they cannot quantify over the
//! PREDICATE position — so the gap is not covered by extending either.
//!
//! * [`super::rdfs`] is **pattern-directed**: it rewrites `?x rdf:type <Class>`
//!   into a union over subclasses. That works only when the pattern NAMES the
//!   thing to expand. The W3C case `rdfs01` asks `SELECT ?x WHERE { ex:a ?x
//!   ex:c }` over `ex:a ex:b1 ex:c` + `ex:b1 rdfs:subPropertyOf ex:b2`, and
//!   expects `{b1, b2}`. You cannot rewrite `?x` into a union over
//!   super-properties: the query is asking which predicates EXIST, so the
//!   entailed triple has to exist to be matched.
//! * `crate::reasoner` declares `Atom { predicate: String, .. }` — a constant
//!   IRI, with only the ARGUMENTS able to be variables. `rdfs9` is expressible
//!   (`type(?s,?c2) :- type(?s,?c1), subClassOf(?c1,?c2)`); rules 2, 3 and 7
//!   are not, because each needs `?p` in predicate position. And rdfs9 is
//!   precisely the rule [`super::rdfs`] already implements — so the two
//!   mechanisms are the same coverage reached two ways, stopping at one wall.
//!
//! So the closure is computed here and WRITTEN, rather than expressed as a
//! query rewrite or a rule.
//!
//! ## Where the derivations go
//!
//! Into the graph's companion inferred graph (`quipu-0b6`), never beside their
//! premises: an overlay's companion is its own, so a closure cannot leak into a
//! parent graph or retract a sibling's output. Premises are read from the graph
//! PLUS its companion, so a derivation can feed another.
//!
//! ## Which rules
//!
//! The four that the failing W3C cases need, plus the two transitive closures
//! they rest on:
//!
//! | rule | derives |
//! |---|---|
//! | rdfs2  | `?s ?p ?o` + `?p rdfs:domain ?c`         -> `?s rdf:type ?c` |
//! | rdfs3  | `?s ?p ?o` + `?p rdfs:range ?c`          -> `?o rdf:type ?c` |
//! | rdfs5  | `?p subPropertyOf ?q` + `?q subPropertyOf ?r` -> `?p subPropertyOf ?r` |
//! | rdfs7  | `?s ?p ?o` + `?p rdfs:subPropertyOf ?q`  -> `?s ?q ?o` |
//! | rdfs9  | `?s rdf:type ?c1` + `?c1 subClassOf ?c2` -> `?s rdf:type ?c2` |
//! | rdfs11 | `?c1 subClassOf ?c2` + `?c2 subClassOf ?c3` -> `?c1 subClassOf ?c3` |
//!
//! Iterated to a fixed point, because rdfs2/rdfs3 produce types that rdfs9 then
//! closes, and rdfs7 produces triples that rdfs2/rdfs3 then read.
//!
//! # Literal-valued premises
//!
//! Both rules that take a literal-valued triple as a PREMISE now fire over one
//! (aegis-x9bmhf):
//!
//! ```text
//! ex:a ex:name "n" .  + ex:name rdfs:domain ex:Person  |= ex:a rdf:type ex:Person   (rdfs2)
//! ex:a ex:b1   "n" .  + ex:b1 rdfs:subPropertyOf ex:b2 |= ex:a ex:b2 "n"            (rdfs7)
//! ```
//!
//! Neither did before: `load()` kept only `Value::Ref` objects, so a
//! literal-valued triple never entered the working set at all. The W3C
//! entailment suite exercises none of this, so the score was unaffected in both
//! directions — which is why it needed finding by review rather than by CI, and
//! why `rdfs:domain` on a datatype property, one of the commonest inferences in
//! real RDF, silently derived nothing.
//!
//! ## What carrying a literal cost
//!
//! rdfs7's conclusion COPIES the object, so the working set had to hold
//! literals. `Obj` (private to this module) is that: an IRI id, or a literal
//! as its SERIALISED BYTES.
//! Bytes rather than a [`crate::types::Value`] because `known` and `fresh` are
//! `BTreeSet`s and `Value` derives neither `Ord` nor `Eq` — it carries
//! `Float(f64)`, which has no total order. Serialised form is already how the
//! store orders values, so this needed no NaN decision and no change to
//! `Value`. Implementing `Ord` on `Value` was the other route and would have
//! forced one, with blast radius well beyond this module.
//!
//! **The rules that may NOT range over a literal guard themselves**, via
//! `Obj::as_ref_id`, rather than relying on the loader to have excluded them:
//! rdfs3 types the OBJECT and rdfs9 reads it as a CLASS, and a literal can be
//! neither. That guard is the one to keep intact — widening the premise set
//! without it silently starts typing literals, which is the obvious wrong fix.
//!
//! Measured cost of the wider working set, on a store that derives NOTHING from
//! it — the case that gains nothing and pays anyway: **~30 ms per 20 000 literal
//! premises**, linear, dominated by the load rather than the rule loop. See the
//! `bench_literal_heavy_closure*` tests, which are `#[ignore]`d and meant to be
//! re-run by whoever next changes the working set.
//!
//! Note for anyone extending this: the `closure_of()` helper in the tests is
//! IRI-only, so an rdfs7 derivation whose object is a literal is INVISIBLE to
//! it. Count `report.asserted`, or read the companion graph directly — both
//! arms exist in the tests.

use std::collections::BTreeSet;

use crate::error::Result;
use crate::namespace::{RDF_TYPE, RDFS_DOMAIN, RDFS_RANGE, RDFS_SUBCLASS_OF, RDFS_SUBPROPERTY_OF};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

/// What one closure run derived.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClosureReport {
    /// Triples written into the companion graph.
    pub asserted: usize,
    /// Fixed-point iterations actually executed.
    pub rounds: usize,
}

impl ClosureReport {
    /// Did this run derive anything?
    ///
    /// Distinct from "it ran": a graph with no schema triples closes to itself
    /// in one round and asserts nothing, which is a correct empty answer rather
    /// than a failure.
    #[must_use]
    pub fn derived_anything(&self) -> bool {
        self.asserted > 0
    }
}

/// One `(subject, predicate, object)` of interned ids. Objects are IRIs only —
/// RDFS entailment ranges over resources, and a literal object cannot be the
/// subject of a derived `rdf:type`.
type Triple = (i64, i64, Obj);

/// An object position: an IRI, or a literal held as its SERIALISED BYTES.
///
/// The bytes rather than a [`Value`] because `known` and `fresh` are
/// `BTreeSet`s and `Value` derives neither `Ord` nor `Eq` — it carries
/// `Float(f64)`, which has no total order. Serialised form is already how the
/// store orders values, so this needs no decision about NaN and no change to
/// `Value`; implementing `Ord` on `Value` would have forced one, with blast
/// radius well beyond this module.
///
/// Cloned into the set, so a literal object costs its own bytes per derived
/// triple. That is the price of rdfs7 carrying literals at all.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Obj {
    /// An IRI, as its interned id.
    Ref(i64),
    /// A literal, as the exact bytes the store holds.
    Lit(Vec<u8>),
}

impl Obj {
    /// The interned id, for the rules that may only range over resources.
    const fn as_ref_id(&self) -> Option<i64> {
        match self {
            Self::Ref(id) => Some(*id),
            Self::Lit(_) => None,
        }
    }

    /// Back to a storable value.
    fn into_value(self) -> Result<Value> {
        match self {
            Self::Ref(id) => Ok(Value::Ref(id)),
            Self::Lit(bytes) => Value::from_bytes(&bytes),
        }
    }
}

/// Which RDFS rule derived a triple. Carried so each derivation is written
/// with its own `reasoner:<rule>` provenance — the store REFUSES any other
/// source into a companion graph (`source_may_write_inferred`), and per-rule
/// tagging matches what the Datalog reasoner already does, so a reader can see
/// WHICH rule produced a triple rather than only that something did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Rule {
    /// domain
    Rdfs2,
    /// range
    Rdfs3,
    /// subPropertyOf transitivity
    Rdfs5,
    /// subPropertyOf
    Rdfs7,
    /// subClassOf
    Rdfs9,
    /// subClassOf transitivity
    Rdfs11,
}

impl Rule {
    const fn source(self) -> &'static str {
        match self {
            Self::Rdfs2 => "reasoner:rdfs2",
            Self::Rdfs3 => "reasoner:rdfs3",
            Self::Rdfs5 => "reasoner:rdfs5",
            Self::Rdfs7 => "reasoner:rdfs7",
            Self::Rdfs9 => "reasoner:rdfs9",
            Self::Rdfs11 => "reasoner:rdfs11",
        }
    }
}

/// Read every triple visible in `graphs`, literal objects included.
///
/// Literals enter the working set because rdfs7 copies the object into its
/// conclusion. Rules that may only range over resources — rdfs3, rdfs9, and the
/// schema slices — filter with [`Obj::as_ref_id`] rather than relying on the
/// loader to have excluded them.
fn load(store: &Store, graphs: &[i64]) -> Result<BTreeSet<Triple>> {
    let mut out = BTreeSet::new();
    let placeholders = graphs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT e, a, v FROM facts \
         WHERE g IN ({placeholders}) AND op = 1 AND valid_to IS NULL"
    );
    let mut stmt = store.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> =
        graphs.iter().map(|g| g as &dyn rusqlite::ToSql).collect();
    let rows = stmt.query_map(params.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    for row in rows {
        let (s, p, raw) = row?;
        // Only Ref-valued objects participate: a literal cannot be a class, a
        // property, or the subject of a derived type.
        // EVERY object, literal included: rdfs7 copies the object into its
        // conclusion, so a literal-valued triple is a premise like any other.
        // The rules that may not range over literals guard themselves below.
        match Value::from_bytes(&raw) {
            Ok(Value::Ref(o)) => {
                out.insert((s, p, Obj::Ref(o)));
            }
            Ok(_) => {
                out.insert((s, p, Obj::Lit(raw)));
            }
            Err(_) => {}
        }
    }
    Ok(out)
}

/// Materialise the RDFS closure of `graph` into its companion inferred graph.
///
/// # Errors
///
/// Propagates store errors.
pub fn materialise(store: &mut Store, graph: i64, timestamp: &str) -> Result<ClosureReport> {
    let type_id = store.intern(RDF_TYPE)?;
    let sub_class = store.intern(RDFS_SUBCLASS_OF)?;
    let sub_prop = store.intern(RDFS_SUBPROPERTY_OF)?;
    let domain = store.intern(RDFS_DOMAIN)?;
    let range = store.intern(RDFS_RANGE)?;

    let companion = store.ensure_companion_inferred_graph(graph, timestamp)?;
    let mut known = load(store, &[graph, companion])?;

    let mut report = ClosureReport::default();
    let mut pending: std::collections::BTreeMap<Rule, Vec<Datum>> =
        std::collections::BTreeMap::new();

    loop {
        report.rounds += 1;
        let mut fresh: BTreeSet<(Rule, Triple)> = BTreeSet::new();

        // Schema slices, recomputed each round: rdfs5/rdfs11 grow them.
        let sub_prop_of: Vec<(i64, i64)> = known
            .iter()
            .filter(|(_, p, _)| *p == sub_prop)
            .filter_map(|(s, _, o)| o.as_ref_id().map(|id| (*s, id)))
            .collect();
        let sub_class_of: Vec<(i64, i64)> = known
            .iter()
            .filter(|(_, p, _)| *p == sub_class)
            .filter_map(|(s, _, o)| o.as_ref_id().map(|id| (*s, id)))
            .collect();
        let domains: Vec<(i64, i64)> = known
            .iter()
            .filter(|(_, p, _)| *p == domain)
            .filter_map(|(s, _, o)| o.as_ref_id().map(|id| (*s, id)))
            .collect();
        let ranges: Vec<(i64, i64)> = known
            .iter()
            .filter(|(_, p, _)| *p == range)
            .filter_map(|(s, _, o)| o.as_ref_id().map(|id| (*s, id)))
            .collect();

        let derive = |rule: Rule, t: Triple, fresh: &mut BTreeSet<(Rule, Triple)>| {
            if !known.contains(&t) {
                fresh.insert((rule, t));
            }
        };

        for (s, p, o) in &known {
            let (s, p) = (*s, *p);
            // rdfs7 — the rule neither existing mechanism can express. Its
            // conclusion COPIES the object, so it is the one rule that carries a
            // literal through, and the reason `Obj` exists.
            for &(sub, sup) in &sub_prop_of {
                if p == sub {
                    derive(Rule::Rdfs7, (s, sup, o.clone()), &mut fresh);
                }
            }
            // rdfs2 — types the SUBJECT, so the object's kind is irrelevant and
            // a literal-valued premise counts (aegis-x9bmhf).
            for &(prop, class) in &domains {
                if p == prop {
                    derive(Rule::Rdfs2, (s, type_id, Obj::Ref(class)), &mut fresh);
                }
            }
            // rdfs3 — types the OBJECT, so it may only fire over a resource. A
            // literal cannot be the subject of an `rdf:type`.
            if let Some(o_id) = o.as_ref_id() {
                for &(prop, class) in &ranges {
                    if p == prop {
                        derive(Rule::Rdfs3, (o_id, type_id, Obj::Ref(class)), &mut fresh);
                    }
                }
                // rdfs9 — the object is a CLASS here, so likewise resource-only.
                if p == type_id {
                    for &(sub, sup) in &sub_class_of {
                        if o_id == sub {
                            derive(Rule::Rdfs9, (s, type_id, Obj::Ref(sup)), &mut fresh);
                        }
                    }
                }
            }
        }
        // rdfs5 / rdfs11 — transitivity of the two hierarchies.
        for &(a, b) in &sub_prop_of {
            for &(c, d) in &sub_prop_of {
                if b == c {
                    derive(Rule::Rdfs5, (a, sub_prop, Obj::Ref(d)), &mut fresh);
                }
            }
        }
        for &(a, b) in &sub_class_of {
            for &(c, d) in &sub_class_of {
                if b == c {
                    derive(Rule::Rdfs11, (a, sub_class, Obj::Ref(d)), &mut fresh);
                }
            }
        }

        if fresh.is_empty() {
            break;
        }
        for (rule, (s, p, o)) in &fresh {
            pending.entry(*rule).or_default().push(Datum {
                entity: *s,
                attribute: *p,
                value: o.clone().into_value()?,
                valid_from: timestamp.to_string(),
                valid_to: None,
                op: Op::Assert,
            });
        }
        known.extend(fresh.iter().map(|(_, t)| t.clone()));
    }

    // One transaction per rule, each carrying that rule's provenance. The
    // store refuses any source that is not `reasoner:*` / `owl:materialize`
    // into a companion, which is what makes "who derived this" answerable.
    for (rule, datums) in &pending {
        report.asserted += datums.len();
        store.transact_to_graph(datums, timestamp, None, Some(rule.source()), companion)?;
    }
    Ok(report)
}
