//! Dispatch-inventory tests. Size-exempt (`*_tests.rs`).

use super::*;
use crate::namespace::RDF_TYPE;
use crate::store::Datum;
use crate::types::Op;

const TS: &str = "2026-01-01T00:00:00Z";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

/// Declare an entity of `class` with the given `aegis:` fields. Repeated field
/// names assert repeated values, which is how `governedAt` gets two points.
fn declare(store: &mut Store, iri: &str, class: &str, fields: &[(&str, &str)]) {
    let entity = store.intern(iri).unwrap();
    let mut datums = vec![Datum {
        entity,
        attribute: store.intern(RDF_TYPE).unwrap(),
        value: Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}{class}")).unwrap()),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }];
    for (name, value) in fields {
        let attribute = if *name == "label" {
            store.intern(RDFS_LABEL).unwrap()
        } else {
            store.intern(&format!("{DEFAULT_BASE_NS}{name}")).unwrap()
        };
        datums.push(Datum {
            entity,
            attribute,
            value: Value::Str((*value).to_string()),
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    store.transact(&datums, TS, None, None).unwrap();
}

fn tool_class(store: &mut Store, id: &str, fields: &[(&str, &str)]) {
    let mut all = vec![("label", id), ("dispatchedBy", "harness")];
    all.extend_from_slice(fields);
    declare(store, &format!("http://ex/tool/{id}"), "ToolClass", &all);
}

fn violations(report: &Report) -> Vec<&crate::governance::audit::Discrepancy> {
    report.of(crate::governance::audit::Severity::Violation)
}

fn incompletenesses(report: &Report) -> Vec<&crate::governance::audit::Discrepancy> {
    report.of(crate::governance::audit::Severity::Incompleteness)
}

#[test]
fn an_empty_inventory_is_not_a_clean_bill_of_health() {
    // The single most misleading thing this module could do is pass by having
    // nothing to check.
    let store = Store::open_in_memory().unwrap();
    let report = check(&store).unwrap();
    assert!(
        report.conforms(),
        "an unwritten inventory is not a contradiction"
    );
    assert!(
        incompletenesses(&report)
            .iter()
            .any(|d| d.detail.contains("unwritten one")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn an_executable_class_that_traverses_nothing_and_says_nothing_is_a_violation() {
    // I7: an unknown hole in the dispatch graph.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(&mut store, "Task", &[("executable", "true")]);
    let report = check(&store).unwrap();
    assert!(
        violations(&report)
            .iter()
            .any(|d| d.detail.contains("unknown hole")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn an_acknowledged_bypass_surface_is_reported_but_is_not_a_violation() {
    // The distinction that is the whole point of writing the list down: an
    // operator cannot tell a decision from an oversight without it.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "ci-pipeline",
        &[
            ("executable", "true"),
            (
                "ungovernedReason",
                "the runner executes the workflow — no agent, no session",
            ),
            ("enforcedInsteadAt", "repo-side checks, branch protection"),
        ],
    );
    let report = check(&store).unwrap();
    assert!(report.conforms(), "{:#?}", report.discrepancies);
    let found = incompletenesses(&report);
    assert!(
        found.iter().any(|d| d.detail.contains("Acknowledged")),
        "{found:#?}"
    );
    assert!(
        found.iter().any(|d| d.detail.contains("branch protection")),
        "names where it IS enforced: {found:#?}"
    );
}

#[test]
fn an_acknowledged_surface_with_no_alternative_says_so() {
    // "Unhandled, and nobody has said where it is handled instead" is a
    // different state from "handled elsewhere", and the report must not blur it.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "hostile-agent",
        &[
            ("executable", "true"),
            ("ungovernedReason", "it can edit its own guard"),
        ],
    );
    let report = check(&store).unwrap();
    assert!(
        incompletenesses(&report)
            .iter()
            .any(|d| d.detail.contains("nothing says where it")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn the_control_a_governed_executable_class_produces_nothing() {
    // Without this, every test above could be passing because the checker flags
    // every class it sees.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "Edit",
        &[("executable", "true"), ("governedAt", "PAG")],
    );
    let report = check(&store).unwrap();
    assert!(report.conforms(), "{:#?}", report.discrepancies);
    assert!(report.is_complete(), "{:#?}", report.discrepancies);
}

#[test]
fn a_read_only_class_needs_no_enforcement_point() {
    // Demanding one would bury the real gaps under the ordinary case.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(&mut store, "Read", &[("executable", "false")]);
    let report = check(&store).unwrap();
    assert!(report.conforms(), "{:#?}", report.discrepancies);
    assert!(report.is_complete(), "{:#?}", report.discrepancies);
}

#[test]
fn an_undeclared_executable_flag_is_undecidable_not_assumed_safe() {
    // Guessing either way is wrong in a direction that matters.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(&mut store, "Mystery", &[]);
    let report = check(&store).unwrap();
    assert!(report.conforms());
    assert!(
        incompletenesses(&report)
            .iter()
            .any(|d| d.detail.contains("undecidable")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn a_class_with_two_points_keeps_both() {
    // A tool call can pass a pre-action gate and a post-action auditor both, and
    // recording whichever row bound last would silently halve the coverage set.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "Edit",
        &[
            ("executable", "true"),
            ("governedAt", "PAG"),
            ("governedAt", "PAA"),
        ],
    );
    let classes = load(&store).unwrap();
    assert_eq!(classes.len(), 1);
    assert_eq!(
        classes[0]
            .governed_at
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["PAA", "PAG"]
    );
}

#[test]
fn a_constraint_placed_where_nothing_traverses_can_never_fire() {
    // The cross-check the other direction: governance in the catalog, inert in
    // the deployment — the failure hardest to see from either side alone.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "Edit",
        &[("executable", "true"), ("governedAt", "PAG")],
    );
    declare(
        &mut store,
        "http://ex/policy/mid-flight",
        "Policy",
        &[
            ("label", "budget-overrun"),
            ("boundary", "action"),
            ("constraintClass", "soft"),
            ("verificationPoint", "ATM"),
        ],
    );
    let report = check(&store).unwrap();
    assert!(
        violations(&report)
            .iter()
            .any(|d| d.detail.contains("can never fire")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn the_control_a_constraint_placed_where_something_traverses_is_fine() {
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "Edit",
        &[("executable", "true"), ("governedAt", "PAG")],
    );
    declare(
        &mut store,
        "http://ex/policy/p",
        "Policy",
        &[
            ("label", "no-ticket"),
            ("boundary", "action"),
            ("constraintClass", "hard"),
            ("verificationPoint", "PAG"),
        ],
    );
    let report = check(&store).unwrap();
    assert!(report.conforms(), "{:#?}", report.discrepancies);
}

#[test]
fn the_shipped_inventory_conforms_to_its_own_check() {
    // The seed file is a claim about this deployment. Shipping one the checker
    // rejects would mean the first thing any operator sees is a red report about
    // our own data.
    let mut store = Store::open_in_memory().unwrap();
    let turtle = std::fs::read_to_string("shapes/dispatch-inventory.ttl")
        .expect("the shipped inventory is part of the repo");
    crate::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        TS,
        None,
        None,
    )
    .unwrap();
    let report = check(&store).unwrap();
    assert!(report.conforms(), "{:#?}", report.discrepancies);
    // And it is not vacuous: the acknowledged bypass surfaces are still there.
    assert!(
        !incompletenesses(&report).is_empty(),
        "the shipped inventory must still admit its ungoverned surfaces"
    );
}

// ── Trust boundaries (SARC §9.5, the zero-trust gateway) ─────────────────────

#[test]
fn an_importing_class_is_reported_even_when_it_is_governed() {
    // The risk is different in kind. `governedAt` says the class's own ACTIONS
    // traverse a point; it says nothing about what the class RETURNED.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "Task",
        &[
            ("executable", "true"),
            ("governedAt", "PAG"),
            ("importsUntrustedState", "true"),
            ("untrustedOrigin", "the sub-agent's response text"),
        ],
    );
    let report = check(&store).unwrap();
    assert!(report.conforms(), "{:#?}", report.discrepancies);
    assert!(
        incompletenesses(&report)
            .iter()
            .any(|d| d.detail.contains("has not been through this")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn an_undocumented_import_channel_is_a_violation() {
    // An import channel nobody can describe is one nobody can weigh.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "mystery-import",
        &[("executable", "false"), ("importsUntrustedState", "true")],
    );
    let report = check(&store).unwrap();
    assert!(
        violations(&report)
            .iter()
            .any(|d| d.detail.contains("nobody can describe")),
        "{:#?}",
        report.discrepancies
    );
}

#[test]
fn the_control_a_non_importing_class_gets_no_trust_finding() {
    let mut store = Store::open_in_memory().unwrap();
    tool_class(
        &mut store,
        "Edit",
        &[("executable", "true"), ("governedAt", "PAG")],
    );
    let report = check(&store).unwrap();
    assert!(report.is_complete(), "{:#?}", report.discrepancies);
}

#[test]
fn an_undeclared_import_flag_is_not_read_as_importing() {
    // Absent is not false and it is not true either — but a class that says
    // nothing about importing has made no claim to check, and inventing one
    // would flag every entry in the file.
    let mut store = Store::open_in_memory().unwrap();
    tool_class(&mut store, "Read", &[("executable", "false")]);
    let report = check(&store).unwrap();
    assert!(report.is_complete(), "{:#?}", report.discrepancies);
}

#[test]
fn the_shipped_inventory_declares_its_import_channels() {
    // The stack really does have them — a sub-agent's response, an MCP server's
    // output, retrieved documents — and a seed file that omitted them would be
    // claiming a closed perimeter this deployment does not have.
    let mut store = Store::open_in_memory().unwrap();
    let turtle = std::fs::read_to_string("shapes/dispatch-inventory.ttl").unwrap();
    crate::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        TS,
        None,
        None,
    )
    .unwrap();
    let importing: Vec<_> = load(&store)
        .unwrap()
        .into_iter()
        .filter(|c| c.imports_untrusted_state == Some(true))
        .collect();
    assert!(
        importing.len() >= 3,
        "{:?}",
        importing.iter().map(|c| &c.label).collect::<Vec<_>>()
    );
    assert!(
        importing.iter().all(|c| c.untrusted_origin.is_some()),
        "every declared import channel must say what enters through it"
    );
    // And the shipped file still passes its own check.
    assert!(check(&store).unwrap().conforms());
}
