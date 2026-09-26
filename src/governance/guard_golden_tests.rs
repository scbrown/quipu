//! Golden characterization of the live write gate (aegis-xfuch4.2).
//!
//! Captured from the gate AS IT STOOD before the evaluator was extracted into
//! one shared `judge` core, and committed ahead of that refactor. After the
//! refactor "gate == judge" is vacuous (the same function on both sides), so
//! these goldens are the only proof that behaviour was preserved: the refactor
//! commit must reproduce `testdata/gate_goldens.txt` byte for byte.
//!
//! Each case builds the post-state the gate would see inside its savepoint (the
//! write is committed with enforcement OFF), then calls
//! [`PolicyRegistry::evaluate_write`] exactly as `Store::enforce_write_policies`
//! does, and renders the result, the staged verdicts and the staged requests.
//! Request `now` is wall-clock, so it is rendered only as "now>0".
//!
//! The corpus covers every branch of the evaluator, including the three places
//! `policy backtest` has drifted from it (non-blocking effects, the evidence
//! probe, graph-scoped type resolution).
//!
//! Regenerate ONLY when a behaviour change is intended and reviewed:
//! `QUIPU_BLESS_GOLDENS=1 cargo test gate_golden`.

use super::PolicyRegistry;
use crate::error::Error;
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

const TS: &str = "2026-01-01T00:00:00Z";
const LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const EVIDENCE: &str = "http://ex/evidence";
const REQUIRE_LABEL: &str = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";
const REQUIRE_COLOR: &str = "ASK { $target <http://ex/color> ?c }";
const HAS_EVIDENCE: &str = "ASK { $target <http://ex/evidence> ?e }";
/// Mint time for fixture `DecisionRequest`s: fixed, so `expiresAt` is stable.
const MINT_NOW: i64 = 1_700_000_000;
const FAR_WINDOW: i64 = 10_000_000_000;
const GOLDEN_PATH: &str = "src/governance/testdata/gate_goldens.txt";

fn datum(store: &Store, s: &str, p: &str, v: Value, op: Op) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: TS.to_string(),
        valid_to: None,
        op,
    }
}

fn a(store: &Store, s: &str, p: &str, v: Value) -> Datum {
    datum(store, s, p, v, Op::Assert)
}

fn iri(store: &Store, s: &str) -> Value {
    Value::Ref(store.intern(s).unwrap())
}

fn lit(s: &str) -> Value {
    Value::Str(s.to_string())
}

fn ns(local: &str) -> String {
    format!("{DEFAULT_BASE_NS}{local}")
}

/// One policy as the fixture defines it.
pub(super) struct Pol<'a> {
    iri: &'a str,
    target: &'a str,
    claim: &'a str,
    effect: Option<&'a str>,
    probe: Option<&'a str>,
    window: Option<i64>,
    exemplar: Option<&'a str>,
}

impl<'a> Pol<'a> {
    fn new(iri: &'a str, target: &'a str, claim: &'a str, effect: Option<&'a str>) -> Self {
        Self {
            iri,
            target,
            claim,
            effect,
            probe: None,
            window: None,
            exemplar: None,
        }
    }
}

fn define(store: &mut Store, p: &Pol<'_>) {
    let mut d = vec![
        a(store, p.iri, RDF_TYPE, iri(store, &ns("Policy"))),
        a(store, p.iri, &ns("targets"), lit(p.target)),
        a(store, p.iri, &ns("claim"), lit(p.claim)),
        a(store, p.iri, &ns("boundary"), lit("action")),
    ];
    if let Some(e) = p.effect {
        d.push(a(store, p.iri, &ns("effect"), lit(e)));
    }
    if let Some(pr) = p.probe {
        d.push(a(store, p.iri, &ns("evidenceProbe"), lit(pr)));
    }
    if let Some(w) = p.window {
        d.push(a(
            store,
            p.iri,
            &ns("reversibilityWindowSeconds"),
            Value::Int(w),
        ));
    }
    if let Some(x) = p.exemplar {
        d.push(a(store, p.iri, &ns("exemplar"), lit(x)));
    }
    store.transact(&d, TS, None, None).unwrap();
}

fn keypair() -> ring::signature::Ed25519KeyPair {
    let rng = ring::rand::SystemRandom::new();
    let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    ring::signature::Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
}

/// Mint a request for (policy, target) at the fixed mint time.
fn open_request(store: &mut Store, policy: &str, target: &str, window: i64) {
    let d = super::super::router::mint_request(store, policy, target, None, window, MINT_NOW, TS)
        .unwrap();
    store.transact(&d, TS, None, None).unwrap();
}

/// Record a properly attested ruling for (policy, target).
fn rule(store: &mut Store, policy: &str, target: &str, outcome: &str, by: &str) {
    use super::super::router::{decision_message, evidence_hash};
    let kp = keypair();
    let hash = evidence_hash(policy, target);
    let reg = format!("http://ex/reg_{by}");
    let d = vec![
        a(
            store,
            &reg,
            RDF_TYPE,
            iri(store, &ns("VerifierRegistration")),
        ),
        a(store, &reg, &ns("verifier"), lit(by)),
        a(store, &reg, &ns("attests"), lit(policy)),
        a(
            store,
            &reg,
            &ns("publicKey"),
            lit(&crate::signing::public_key_hex(&kp)),
        ),
    ];
    store.transact(&d, TS, None, None).unwrap();
    let sig = crate::signing::sign_hex(&kp, &decision_message(&hash, outcome, by));
    let dec = format!("http://ex/decision_{by}");
    let d = vec![
        a(store, &dec, RDF_TYPE, iri(store, &ns("Decision"))),
        a(store, &dec, &ns("outcome"), lit(outcome)),
        a(store, &dec, &ns("by"), lit(by)),
        a(store, &dec, &ns("evidenceHash"), lit(&hash)),
        a(store, &dec, &ns("signature"), lit(&sig)),
    ];
    store.transact(&d, TS, None, None).unwrap();
}

/// The policies of the fixture. Each case targets its own type, so cases do not
/// interfere, except where a case deliberately exercises several.
pub(super) fn policies() -> Vec<Pol<'static>> {
    let mut v = vec![
        Pol::new(
            "http://ex/P_deny",
            "http://ex/TDeny",
            REQUIRE_LABEL,
            Some("deny"),
        ),
        Pol::new(
            "http://ex/P_default",
            "http://ex/TDefault",
            REQUIRE_LABEL,
            None,
        ),
        Pol::new(
            "http://ex/P_warn",
            "http://ex/TWarn",
            REQUIRE_LABEL,
            Some("warn"),
        ),
        Pol::new(
            "http://ex/P_allow",
            "http://ex/TAllow",
            REQUIRE_LABEL,
            Some("allow"),
        ),
        Pol::new(
            "http://ex/P_record",
            "http://ex/TRecord",
            REQUIRE_LABEL,
            Some("record"),
        ),
        Pol::new(
            "http://ex/P_throttle",
            "http://ex/TThrottle",
            REQUIRE_LABEL,
            Some("throttle"),
        ),
        Pol::new(
            "http://ex/P_multi_a",
            "http://ex/TMulti",
            REQUIRE_LABEL,
            Some("deny"),
        ),
        Pol::new(
            "http://ex/P_multi_b",
            "http://ex/TMulti",
            REQUIRE_COLOR,
            Some("deny"),
        ),
        Pol::new(
            "http://ex/P_mixed_warn",
            "http://ex/TMixed",
            REQUIRE_COLOR,
            Some("warn"),
        ),
        Pol::new(
            "http://ex/P_mixed_deny",
            "http://ex/TMixed",
            REQUIRE_LABEL,
            Some("deny"),
        ),
        Pol::new(
            "http://ex/P_second_type",
            "http://ex/TSecond",
            REQUIRE_COLOR,
            Some("deny"),
        ),
    ];
    let mut probe = Pol::new(
        "http://ex/P_probe",
        "http://ex/TProbe",
        REQUIRE_LABEL,
        Some("deny"),
    );
    probe.probe = Some(HAS_EVIDENCE);
    v.push(probe);
    let mut ex = Pol::new(
        "http://ex/P_ex",
        "http://ex/TEx",
        REQUIRE_LABEL,
        Some("deny"),
    );
    ex.exemplar = Some("http://ex/verdict_motivating");
    v.push(ex);
    for (p, t, e) in [
        ("http://ex/P_ra", "http://ex/TRa", "require-approval"),
        ("http://ex/P_esc", "http://ex/TEsc", "escalate"),
    ] {
        let mut pol = Pol::new(p, t, REQUIRE_LABEL, Some(e));
        pol.window = Some(3600);
        v.push(pol);
    }
    let mut nowin = Pol::new(
        "http://ex/P_nowin",
        "http://ex/TNoWin",
        REQUIRE_LABEL,
        Some("escalate"),
    );
    nowin.window = None;
    v.push(nowin);
    v
}

/// Graph ids used by the graph-scope cases.
const G_OTHER: i64 = 7;
const G_WRITE: i64 = 9;

/// One golden case: a name, the setup that builds the post-state, and the
/// datums + graph the gate is asked about.
struct Case {
    name: &'static str,
    graph: i64,
    build: fn(&mut Store) -> Vec<Datum>,
}

fn typed(store: &Store, e: &str, t: &str) -> Datum {
    a(store, e, RDF_TYPE, iri(store, t))
}

/// Commit `d` into ROOT (enforcement is off) and return it as the gate's input.
fn commit(store: &mut Store, d: Vec<Datum>) -> Vec<Datum> {
    store.transact(&d, TS, None, None).unwrap();
    d
}

fn commit_to(store: &mut Store, d: Vec<Datum>, g: i64) -> Vec<Datum> {
    store.transact_to_graph(&d, TS, None, None, g).unwrap();
    d
}

#[allow(clippy::too_many_lines)]
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "deny_unsatisfied",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_deny_u", "http://ex/TDeny")];
                commit(s, d)
            },
        },
        Case {
            name: "deny_satisfied",
            graph: 0,
            build: |s| {
                let d = vec![
                    typed(s, "http://ex/e_deny_s", "http://ex/TDeny"),
                    a(s, "http://ex/e_deny_s", LABEL, lit("x")),
                ];
                commit(s, d)
            },
        },
        Case {
            name: "default_effect_is_deny",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_default", "http://ex/TDefault")];
                commit(s, d)
            },
        },
        Case {
            name: "warn_is_not_evaluated",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_warn", "http://ex/TWarn")];
                commit(s, d)
            },
        },
        Case {
            name: "allow_is_not_evaluated",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_allow", "http://ex/TAllow")];
                commit(s, d)
            },
        },
        Case {
            name: "record_is_not_evaluated",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_record", "http://ex/TRecord")];
                commit(s, d)
            },
        },
        Case {
            name: "throttle_is_not_evaluated",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_throttle", "http://ex/TThrottle")];
                commit(s, d)
            },
        },
        Case {
            name: "ungoverned_type",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_note", "http://ex/TNote")];
                commit(s, d)
            },
        },
        Case {
            name: "untyped_entity",
            graph: 0,
            build: |s| {
                let d = vec![a(s, "http://ex/e_untyped", LABEL, lit("x"))];
                commit(s, d)
            },
        },
        Case {
            name: "probe_false_is_unknown",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_probe_f", "http://ex/TProbe")];
                commit(s, d)
            },
        },
        Case {
            name: "probe_true_unsatisfied",
            graph: 0,
            build: |s| {
                let d = vec![
                    typed(s, "http://ex/e_probe_tu", "http://ex/TProbe"),
                    a(s, "http://ex/e_probe_tu", EVIDENCE, lit("y")),
                ];
                commit(s, d)
            },
        },
        Case {
            name: "probe_true_satisfied",
            graph: 0,
            build: |s| {
                let d = vec![
                    typed(s, "http://ex/e_probe_ts", "http://ex/TProbe"),
                    a(s, "http://ex/e_probe_ts", EVIDENCE, lit("y")),
                    a(s, "http://ex/e_probe_ts", LABEL, lit("x")),
                ];
                commit(s, d)
            },
        },
        Case {
            name: "exemplar_cited",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_ex", "http://ex/TEx")];
                commit(s, d)
            },
        },
        Case {
            name: "multi_first_fails",
            graph: 0,
            build: |s| {
                let d = vec![
                    typed(s, "http://ex/e_multi_1", "http://ex/TMulti"),
                    a(s, "http://ex/e_multi_1", "http://ex/color", lit("red")),
                ];
                commit(s, d)
            },
        },
        Case {
            name: "multi_second_fails",
            graph: 0,
            build: |s| {
                let d = vec![
                    typed(s, "http://ex/e_multi_2", "http://ex/TMulti"),
                    a(s, "http://ex/e_multi_2", LABEL, lit("x")),
                ];
                commit(s, d)
            },
        },
        Case {
            name: "multi_both_satisfied",
            graph: 0,
            build: |s| {
                let d = vec![
                    typed(s, "http://ex/e_multi_3", "http://ex/TMulti"),
                    a(s, "http://ex/e_multi_3", LABEL, lit("x")),
                    a(s, "http://ex/e_multi_3", "http://ex/color", lit("red")),
                ];
                commit(s, d)
            },
        },
        Case {
            name: "mixed_warn_and_deny",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_mixed", "http://ex/TMixed")];
                commit(s, d)
            },
        },
        Case {
            name: "two_governed_types",
            graph: 0,
            build: |s| {
                let d = vec![
                    typed(s, "http://ex/e_two", "http://ex/TDeny"),
                    typed(s, "http://ex/e_two", "http://ex/TSecond"),
                    a(s, "http://ex/e_two", LABEL, lit("x")),
                ];
                commit(s, d)
            },
        },
        Case {
            name: "two_entities_one_bad",
            graph: 0,
            build: |s| {
                let d = vec![
                    typed(s, "http://ex/e_pair_ok", "http://ex/TDeny"),
                    a(s, "http://ex/e_pair_ok", LABEL, lit("x")),
                    typed(s, "http://ex/e_pair_bad", "http://ex/TDeny"),
                ];
                commit(s, d)
            },
        },
        Case {
            name: "unrelated_edit_of_compliant",
            graph: 0,
            build: |s| {
                commit(
                    s,
                    vec![
                        typed(s, "http://ex/e_edit", "http://ex/TDeny"),
                        a(s, "http://ex/e_edit", LABEL, lit("x")),
                    ],
                );
                commit(
                    s,
                    vec![a(s, "http://ex/e_edit", "http://ex/color", lit("red"))],
                )
            },
        },
        Case {
            name: "retraction_breaks_claim",
            graph: 0,
            build: |s| {
                commit(
                    s,
                    vec![
                        typed(s, "http://ex/e_retract", "http://ex/TDeny"),
                        a(s, "http://ex/e_retract", LABEL, lit("x")),
                    ],
                );
                let r = vec![datum(
                    s,
                    "http://ex/e_retract",
                    LABEL,
                    lit("x"),
                    Op::Retract,
                )];
                commit(s, r)
            },
        },
        // Graph scope: the gate resolves types in the WRITE graph or ROOT only.
        Case {
            name: "type_in_root_write_in_overlay",
            graph: G_WRITE,
            build: |s| {
                commit(s, vec![typed(s, "http://ex/e_g_root", "http://ex/TDeny")]);
                commit_to(
                    s,
                    vec![a(s, "http://ex/e_g_root", "http://ex/color", lit("red"))],
                    G_WRITE,
                )
            },
        },
        Case {
            name: "type_in_other_graph_is_invisible",
            graph: G_WRITE,
            build: |s| {
                commit_to(
                    s,
                    vec![typed(s, "http://ex/e_g_other", "http://ex/TDeny")],
                    G_OTHER,
                );
                commit_to(
                    s,
                    vec![a(s, "http://ex/e_g_other", "http://ex/color", lit("red"))],
                    G_WRITE,
                )
            },
        },
        Case {
            name: "type_in_write_graph",
            graph: G_WRITE,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_g_write", "http://ex/TDeny")];
                commit_to(s, d, G_WRITE)
            },
        },
        // The router arms.
        Case {
            name: "require_approval_no_request",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_ra_none", "http://ex/TRa")];
                commit(s, d)
            },
        },
        Case {
            name: "escalate_no_request",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_esc_none", "http://ex/TEsc")];
                commit(s, d)
            },
        },
        Case {
            name: "escalate_no_window_declared",
            graph: 0,
            build: |s| {
                let d = vec![typed(s, "http://ex/e_nowin", "http://ex/TNoWin")];
                commit(s, d)
            },
        },
        Case {
            name: "require_approval_pending",
            graph: 0,
            build: |s| {
                open_request(s, "http://ex/P_ra", "http://ex/e_ra_pend", FAR_WINDOW);
                let d = vec![typed(s, "http://ex/e_ra_pend", "http://ex/TRa")];
                commit(s, d)
            },
        },
        Case {
            name: "require_approval_expired",
            graph: 0,
            build: |s| {
                open_request(s, "http://ex/P_ra", "http://ex/e_ra_exp", 0);
                let d = vec![typed(s, "http://ex/e_ra_exp", "http://ex/TRa")];
                commit(s, d)
            },
        },
        Case {
            name: "require_approval_approved",
            graph: 0,
            build: |s| {
                open_request(s, "http://ex/P_ra", "http://ex/e_ra_ok", FAR_WINDOW);
                rule(s, "http://ex/P_ra", "http://ex/e_ra_ok", "approve", "alice");
                let d = vec![typed(s, "http://ex/e_ra_ok", "http://ex/TRa")];
                commit(s, d)
            },
        },
        Case {
            name: "require_approval_rejected",
            graph: 0,
            build: |s| {
                open_request(s, "http://ex/P_ra", "http://ex/e_ra_no", FAR_WINDOW);
                rule(s, "http://ex/P_ra", "http://ex/e_ra_no", "reject", "bob");
                let d = vec![typed(s, "http://ex/e_ra_no", "http://ex/TRa")];
                commit(s, d)
            },
        },
        Case {
            name: "escalate_satisfied_skips_router",
            graph: 0,
            build: |s| {
                open_request(s, "http://ex/P_esc", "http://ex/e_esc_sat", 0);
                let d = vec![
                    typed(s, "http://ex/e_esc_sat", "http://ex/TEsc"),
                    a(s, "http://ex/e_esc_sat", LABEL, lit("x")),
                ];
                commit(s, d)
            },
        },
    ]
}

/// Render one case's gate output deterministically.
fn render(
    name: &str,
    result: &crate::error::Result<()>,
    v: &[super::super::verdict_facts::PendingVerdict],
    r: &[super::super::router::PendingRequest],
) -> String {
    let mut out = format!("case {name}\n");
    match result {
        Ok(()) => out.push_str("  result: admitted\n"),
        Err(Error::PolicyDenied(msg)) => out.push_str(&format!("  result: denied: {msg}\n")),
        Err(e) => out.push_str(&format!("  result: error: {e}\n")),
    }
    for x in v {
        out.push_str(&format!(
            "  verdict: {} {} {}\n",
            x.predicate_id, x.target_ref, x.outcome
        ));
    }
    for x in r {
        out.push_str(&format!(
            "  request: {} {} window={} now>0={}\n",
            x.policy_iri,
            x.target_iri,
            x.window_secs,
            x.now > 0
        ));
    }
    out
}

/// Run the whole corpus against the live gate and return the rendered goldens.
/// `perturb` lets a sabotage arm alter the fixture before evaluation.
pub(super) fn run_corpus(perturb: &dyn Fn(&mut Vec<Pol<'static>>)) -> String {
    let mut out = String::new();
    for case in cases() {
        // A fresh store per case: no case sees another's facts or requests.
        let mut store = Store::open_in_memory().unwrap();
        store.governance_config_mut().enforce_on_write = false;
        let mut pols = policies();
        perturb(&mut pols);
        for p in &pols {
            define(&mut store, p);
        }
        let datums = (case.build)(&mut store);
        let registry = PolicyRegistry::build(&store).unwrap();
        let mut verdicts = Vec::new();
        let mut requests = Vec::new();
        let result =
            registry.evaluate_write(&store, &datums, case.graph, &mut verdicts, &mut requests);
        out.push_str(&render(case.name, &result, &verdicts, &requests));
    }
    out
}

fn golden_file() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_PATH)
}

#[test]
fn gate_golden_corpus_is_reproduced_byte_for_byte() {
    let actual = run_corpus(&|_| {});
    let path = golden_file();
    if std::env::var_os("QUIPU_BLESS_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).expect("golden file present");
    assert_eq!(
        actual, expected,
        "the live write gate no longer reproduces its committed goldens. If the \
         change is INTENDED, regenerate with QUIPU_BLESS_GOLDENS=1 and review the diff."
    );
}

/// The corpus must be deterministic, or the byte comparison proves nothing.
#[test]
fn gate_golden_corpus_is_deterministic() {
    assert_eq!(run_corpus(&|_| {}), run_corpus(&|_| {}));
}

/// Lines of `b` that differ from `a`, keyed by the case they belong to.
fn changed_cases(a: &str, b: &str) -> Vec<String> {
    let split = |s: &str| {
        let mut m = std::collections::BTreeMap::new();
        let mut cur = String::new();
        for line in s.lines() {
            if let Some(n) = line.strip_prefix("case ") {
                cur = n.to_string();
            }
            m.entry(cur.clone())
                .or_insert_with(String::new)
                .push_str(line);
        }
        m
    };
    let (ma, mb) = (split(a), split(b));
    ma.keys()
        .filter(|k| ma.get(*k) != mb.get(*k))
        .cloned()
        .collect()
}

/// SABOTAGE: the goldens are not vacuous. Flipping one rule's effect from
/// deny to warn changes EXACTLY the cases that exercise it, and no other.
#[test]
fn sabotage_one_rule_changes_exactly_its_cases() {
    let base = run_corpus(&|_| {});
    let flipped = run_corpus(&|ps| {
        for p in ps.iter_mut() {
            if p.iri == "http://ex/P_default" {
                p.effect = Some("warn");
            }
        }
    });
    assert_eq!(
        changed_cases(&base, &flipped),
        vec!["default_effect_is_deny"]
    );

    // A claim perturbation on the shared deny policy reaches every case that
    // evaluates it, and only those.
    let loosened = run_corpus(&|ps| {
        for p in ps.iter_mut() {
            if p.iri == "http://ex/P_deny" {
                p.claim = "ASK { $target ?p ?o }";
            }
        }
    });
    assert_eq!(
        changed_cases(&base, &loosened),
        vec![
            "deny_unsatisfied",
            "retraction_breaks_claim",
            "two_entities_one_bad",
            // NOT type_in_write_graph: the gate resolves TYPES in the write
            // graph or ROOT, but a claim is a default-graph ASK, which reads
            // ROOT only. An entity typed only in an overlay therefore fails
            // even a claim that is true of every subject. This arm is what
            // measured that asymmetry; the shadow gate must reproduce it.
            "type_in_root_write_in_overlay",
        ]
    );
}
