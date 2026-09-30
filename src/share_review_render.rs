//! Markdown rendering of a [`Review`] for a PR check-run summary.
use super::{Review, ShaclReview};
use crate::share_diff::render_markdown;

/// Inline code, safe inside a table cell.
fn code(v: &str) -> String {
    format!("`{}`", cell(&v.replace('`', "'")))
}

fn cell(v: &str) -> String {
    v.replace('|', "\\|").replace(['\n', '\r'], " ")
}

fn list(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".into()
    } else {
        values
            .iter()
            .map(|v| code(v))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn facts(r: &Review, s: &mut String) {
    s.push_str("### Facts\n\n");
    // The diff renders entities as `###`; nest them under this section.
    for line in render_markdown(&r.diff).lines() {
        if line.starts_with("### ") {
            s.push('#');
        }
        s.push_str(line);
        s.push('\n');
    }
    s.push('\n');
}

fn shacl(r: &Review, s: &mut String) {
    s.push_str("### SHACL violations introduced\n\n");
    match &r.shacl {
        ShaclReview::NotChecked { reason, .. } => s.push_str(&format!(
            "**NOT CHECKED**: {reason}. This is not a zero: no violation count was computed.\n\n"
        )),
        ShaclReview::Checked {
            introduced,
            introduced_count,
            preexisting,
            resolved,
            shapes_changed,
        } => {
            s.push_str(&format!(
                "**{introduced_count} introduced.** Old data was validated against the old \
                 pack's shapes and new data against the new pack's shapes; a violation \
                 already present in old is not counted. Pre-existing: {preexisting}. \
                 Resolved by this change: {resolved}. Shapes changed: {}.\n\n",
                if *shapes_changed { "yes" } else { "no" }
            ));
            if !introduced.is_empty() {
                s.push_str("| Focus | Path | Constraint | Value | Count | Cause | Message |\n");
                s.push_str("|---|---|---|---|---|---|---|\n");
                for v in introduced {
                    s.push_str(&format!(
                        "| {} | {} | {} | {} | {} | {} | {} |\n",
                        cell(&v.focus),
                        v.path.as_deref().map_or_else(String::new, cell),
                        cell(&v.constraint),
                        v.value.as_deref().map_or_else(String::new, code),
                        v.count,
                        if v.from_shapes_change {
                            "shapes change"
                        } else {
                            "data change"
                        },
                        v.message.as_deref().map_or_else(String::new, cell),
                    ));
                }
                s.push('\n');
            }
        }
    }
}

fn decisions(r: &Review, s: &mut String) {
    s.push_str("### Merge decisions\n\n");
    let Some(d) = &r.decisions else {
        s.push_str("No `decisions.json` sidecar accompanies this change.\n\n");
        return;
    };
    s.push_str(&format!(
        "{} conflict(s), {} alias proposal(s), {} unresolved.\n\n",
        d.conflicts.len(),
        d.aliases.len(),
        d.unresolved
    ));
    let resolved = |r: &Option<String>| {
        r.as_deref()
            .map_or_else(|| "**UNRESOLVED**".to_string(), |v| format!("**{v}**"))
    };
    for c in &d.conflicts {
        s.push_str(&format!(
            "- `{}` {} / **{}**: conflict because `{}`; base {}; ours {}; theirs {}; resolution {}\n",
            c.key,
            c.subject,
            c.predicate,
            c.constraint,
            list(&c.base),
            list(&c.ours),
            list(&c.theirs),
            resolved(&c.resolution)
        ));
    }
    for a in &d.aliases {
        s.push_str(&format!(
            "- `{}` alias proposal: {} and {} (similarity {:.3}); resolution {}\n",
            a.key,
            a.ours,
            a.theirs,
            a.similarity,
            resolved(&a.resolution)
        ));
    }
    s.push('\n');
}

fn aliases(r: &Review, s: &mut String) {
    s.push_str("### Alias caveat\n\n");
    s.push_str(
        "This review compares triples. It cannot see two different IRIs minted for one \
         real entity: such a pair diffs, merges and validates cleanly as two entities. \
         The candidates below are advisory (same `rdf:type`, normalized-label \
         Jaro-Winkler at least 0.90, the merge driver's proposer); their absence is not \
         evidence that no alias was introduced.\n\n",
    );
    if r.alias_candidates.is_empty() {
        s.push_str("No candidate pairs proposed.\n\n");
        return;
    }
    for c in &r.alias_candidates {
        s.push_str(&format!(
            "- added {} resembles {} {} (similarity {:.3})\n",
            c.added,
            if c.other_in_old {
                "existing"
            } else {
                "also-added"
            },
            c.other,
            c.similarity
        ));
    }
    s.push('\n');
}

/// The PR-review report. Its LAST LINE is always [`Review::summary`], so a
/// caller can use it as a check-run title without parsing the rest.
pub fn render_report_markdown(r: &Review) -> String {
    let mut s = String::new();
    facts(r, &mut s);
    shacl(r, &mut s);
    decisions(r, &mut s);
    aliases(r, &mut s);
    s.push_str("### Summary\n\n");
    s.push_str(&r.summary);
    s.push('\n');
    s
}
