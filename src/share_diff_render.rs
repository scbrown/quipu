//! Renderings of a [`PackDiff`] and of one [`Snapshot`] (aegis-fxpbys.1).
use std::collections::BTreeMap;

use super::{EntityStatus, Fact, Names, PackDiff, Snapshot, times};

fn graph_suffix(graph: &Option<String>) -> String {
    graph
        .as_ref()
        .map(|g| format!("  @ {g}"))
        .unwrap_or_default()
}

fn marker(status: EntityStatus) -> char {
    match status {
        EntityStatus::Added => '+',
        EntityStatus::Removed => '-',
        EntityStatus::Modified => '~',
    }
}

/// Plain-text rendering: one header per entity, one line per fact.
pub fn render_text(d: &PackDiff) -> String {
    let mut s = String::new();
    for e in &d.entities {
        s.push_str(&format!("{} {}\n", marker(e.status), e.name));
        for c in &e.changed {
            s.push_str(&format!(
                "  ~ {}: {} -> {}{}\n",
                c.predicate,
                c.old,
                c.new,
                graph_suffix(&c.graph)
            ));
        }
        for (sign, facts) in [('-', &e.removed), ('+', &e.added)] {
            for f in facts {
                s.push_str(&format!(
                    "  {sign} {}: {}{}\n",
                    f.predicate,
                    f.value,
                    graph_suffix(&f.graph)
                ));
            }
        }
    }
    s.push_str(&summary(d));
    s
}

fn summary(d: &PackDiff) -> String {
    if d.entities.is_empty() {
        return "no semantic changes\n".into();
    }
    format!(
        "{} {}: {} changed, {} added, {} removed facts\n",
        d.entities.len(),
        if d.entities.len() == 1 {
            "entity"
        } else {
            "entities"
        },
        d.changed,
        d.added,
        d.removed
    )
}

/// Markdown rendering for a PR comment or release note.
pub fn render_markdown(d: &PackDiff) -> String {
    let code = |v: &str| format!("`{}`", v.replace('`', "'"));
    let mut s = String::new();
    for e in &d.entities {
        s.push_str(&format!("### {} {}\n\n", marker(e.status), e.name));
        for c in &e.changed {
            s.push_str(&format!(
                "- **{}**: {} -> {}{}\n",
                c.predicate,
                code(&c.old),
                code(&c.new),
                graph_suffix(&c.graph)
            ));
        }
        for (sign, facts) in [("removed", &e.removed), ("added", &e.added)] {
            for f in facts {
                s.push_str(&format!(
                    "- {sign} **{}**: {}{}\n",
                    f.predicate,
                    code(&f.value),
                    graph_suffix(&f.graph)
                ));
            }
        }
        s.push('\n');
    }
    s.push_str(&summary(d));
    s
}

/// One payload as stable, labelled, entity-grouped text, for `git diff`'s
/// `textconv`. Entities sort by key (never by label, so a relabel does not
/// reorder the file) and facts by displayed predicate and value.
pub fn render_textconv(snap: &Snapshot) -> String {
    let names = Names {
        labels: vec![&snap.labels],
    };
    let mut by_subject: BTreeMap<&str, Vec<(&Fact, usize)>> = BTreeMap::new();
    for (f, n) in &snap.facts {
        by_subject.entry(&f.subject).or_default().push((f, *n));
    }
    let by_subject = by_subject.into_iter().map(|(subject, facts)| {
        let preds = names.preds(facts.iter().map(|(f, _)| f.predicate.as_str()));
        let lines: Vec<String> = facts
            .iter()
            .map(|(f, n)| {
                let graph = (!f.graph.is_empty()).then(|| names.term(&f.graph, &[snap]));
                format!(
                    "  {}: {}{}{}",
                    preds[f.predicate.as_str()],
                    names.term(&f.object, &[snap]),
                    times(*n),
                    graph_suffix(&graph)
                )
            })
            .collect();
        (subject, lines)
    });
    // IRIs (`<`) before blank-node entities (`_`) by ASCII order.
    let mut out = String::new();
    for (subject, mut lines) in by_subject {
        lines.sort_unstable();
        lines.dedup();
        out.push_str(&names.entity(subject, &[snap]));
        out.push('\n');
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}
