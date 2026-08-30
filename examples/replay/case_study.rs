//! End-to-end share, divergence, and reconnect case study.

use super::*;

/// ARM C — the share / diverge / reconnect case study, walked end to end on a
/// real bundle so a reader can audit every hop. Each step prints the identifier
/// the next step consumes, which is the only way to show the lineage is real
/// rather than asserted.
pub(super) fn run(corpus: &Corpus, tmp: &Path) -> i32 {
    let dir = tmp.join("case-study");
    std::fs::create_dir_all(&dir).expect("case study dir");
    let mut failures = 0;
    let mut step = 0;
    let mut say = |what: &str, detail: String| {
        step += 1;
        println!("  {step}. {what:<34} {detail}");
    };

    // A real slice: the first 40 subjects that production actually doubled.
    let slice: String = corpus
        .comment_doublings
        .iter()
        .take(40)
        .map(|r| {
            iri(&r.subject, TYPE, "https://example.org/kg/Entity")
                + &lit(&r.subject, LABEL, "an entity from the recorded corpus")
                + &lit(&r.subject, COMMENT, &r.values[0])
        })
        .collect();

    let mut origin = Store::open_in_memory().expect("store");
    origin
        .load_shapes("replay", SHAPES, "2026-08-29")
        .expect("shapes");
    ingest_rdf(
        &mut origin,
        slice.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-08-29T00:00:00Z",
        None,
        Some("origin"),
    )
    .expect("ingest");

    let base_dir = dir.join("base");
    let base = share(
        &origin,
        base_dir.to_str().unwrap(),
        &ShareOptions {
            shapes: vec!["replay".into()],
            ..Default::default()
        },
    )
    .expect("base share");
    say(
        "SHARE base bundle",
        format!(
            "share_id {}…  graph_hash {}…",
            &base.share_id[..12],
            &base.graph_hash[..12]
        ),
    );

    // Both peers start from that bundle and diverge without coordinating.
    let peer_edit: String = corpus
        .comment_doublings
        .iter()
        .take(40)
        .map(|r| lit(&r.subject, "https://example.org/kg/reviewedBy", "peer"))
        .collect();
    ingest_rdf(
        &mut origin,
        peer_edit.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-08-29T00:01:00Z",
        None,
        Some("peer"),
    )
    .expect("peer edit");
    let incoming_dir = dir.join("incoming");
    let incoming = share(
        &origin,
        incoming_dir.to_str().unwrap(),
        &ShareOptions {
            shapes: vec!["replay".into()],
            parent_share: Some(base.share_id.clone()),
            ..Default::default()
        },
    )
    .expect("incoming share");
    say(
        "DIVERGE peer publishes",
        format!(
            "share_id {}…  parent {}…",
            &incoming.share_id[..12],
            &base.share_id[..12]
        ),
    );

    let mut local = Store::open_in_memory().expect("store");
    local
        .load_shapes("replay", SHAPES, "2026-08-29")
        .expect("shapes");
    ingest_rdf(
        &mut local,
        slice.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-08-29T00:00:00Z",
        None,
        Some("origin"),
    )
    .expect("ingest");
    let our_edit: String = corpus
        .comment_doublings
        .iter()
        .take(40)
        .map(|r| lit(&r.subject, "https://example.org/kg/owner", "us"))
        .collect();
    ingest_rdf(
        &mut local,
        our_edit.as_bytes(),
        RdfFormat::NTriples,
        None,
        "2026-08-29T00:02:00Z",
        None,
        Some("ours"),
    )
    .expect("our edit");
    say(
        "DIVERGE we edit locally",
        format!("{} triples added on our side", 40),
    );

    let st = status(&local, &incoming_dir).expect("status");
    say(
        "STATUS before reconnect",
        format!(
            "diverged={}  ours+{} theirs+{}  conflicts {}  base found at {}",
            st.diverged,
            st.ours_added,
            st.theirs_added,
            st.conflicts.len(),
            Path::new(&st.base_path)
                .file_name()
                .unwrap()
                .to_string_lossy()
        ),
    );
    if !st.diverged {
        eprintln!("  CASE STUDY FAILED: two independently edited copies must read as diverged");
        failures += 1;
    }

    let result = merge(
        &mut local,
        &incoming_dir,
        "2026-08-29T00:03:00Z",
        Some("kelly"),
    )
    .expect("merge");
    say(
        "RECONNECT merge",
        format!(
            "outcome {}  tx {}  asserted {}  retracted {}",
            result.outcome,
            result.tx_id.map_or("-".to_string(), |i| i.to_string()),
            result.asserted,
            result.retracted
        ),
    );
    if result.outcome != "merged" {
        eprintln!("  CASE STUDY FAILED: disjoint predicates must reconcile without a decision");
        failures += 1;
    }

    // The merge result is itself provenance: one transaction, two parents.
    let tx = local
        .get_transaction(result.tx_id.unwrap())
        .expect("tx")
        .expect("tx present");
    let src = tx.source.clone().unwrap_or_default();
    let both =
        src.contains(&result.provenance_parents[0]) && src.contains(&result.provenance_parents[1]);
    say(
        "PROVENANCE two parents",
        format!(
            "recorded_on_tx={both}  {}… + {}…",
            &result.provenance_parents[0][..12],
            &result.provenance_parents[1][..12]
        ),
    );
    if !both {
        eprintln!("  CASE STUDY FAILED: the merge transaction must name both parents");
        failures += 1;
    }

    // Both sides' work must be present afterwards — a merge that silently
    // drops one side reports success just as loudly as one that does not.
    let (bytes, _) =
        quipu::rdf::export_rdf_subset(&local, RdfFormat::NTriples, None).expect("export");
    let text = String::from_utf8(bytes).expect("utf8");
    let ours_kept = text.matches("/owner>").count();
    let theirs_kept = text.matches("/reviewedBy>").count();
    say(
        "CONVERGED both sides kept",
        format!("ours {ours_kept}/40  theirs {theirs_kept}/40"),
    );
    if ours_kept != 40 || theirs_kept != 40 {
        eprintln!("  CASE STUDY FAILED: a side's work was lost by the merge");
        failures += 1;
    }

    // `status.theirs_added` compares the INCOMING BUNDLE with the BASE BUNDLE,
    // so it is unchanged by a local merge and is not a convergence signal —
    // reading it as one is the easy mistake here. What actually settles
    // reconnect is whether anything of theirs is still outstanding LOCALLY.
    let after = status(&local, &incoming_dir).expect("status after");
    let incoming_graph =
        std::fs::read_to_string(incoming_dir.join("export.nt")).expect("incoming export");
    let local_lines: BTreeSet<String> = text.lines().map(|l| l.trim().to_string()).collect();
    let outstanding = incoming_graph
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !local_lines.contains(l))
        .count();
    say(
        "STATUS after reconnect",
        format!(
            "outstanding from theirs {outstanding}  (bundle-level theirs_added stays {}, by design)",
            after.theirs_added
        ),
    );
    if outstanding != 0 {
        eprintln!("  CASE STUDY FAILED: {outstanding} incoming triples never landed locally");
        failures += 1;
    }
    failures
}
