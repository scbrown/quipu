//! The merge strategies under comparison.
//!
//! Every strategy has the same contract: given `(base, ours, theirs)`, produce
//! the graph it would land WITHOUT asking anyone, plus the set of slots it
//! hands to a human. A slot it hands over is held at its base value in the
//! output, so the two headline metrics are not double-counting each other:
//! `conflicts` is what the strategy charged a person for, and the post-merge
//! SHACL violations are the corruption it admitted for free.

use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Stdio};

use crate::generate::set_three_way;
use crate::model::{self, Graph, Slot, Term, Triple, by_slot};
use crate::shapes;

/// What a strategy produced.
pub struct Outcome {
    /// The graph the strategy lands automatically.
    pub merged: Graph,
    /// Slots the strategy refuses to decide.
    pub conflicts: BTreeSet<Slot>,
    /// Output lines that were not well-formed RDF. Non-zero only for the
    /// line-merge arms, and the reason they are in the table.
    pub unparseable_lines: usize,
    /// False when the arm could not run in this environment (no `git`), so an
    /// absent row is never silently read as a zero.
    pub available: bool,
}

impl Outcome {
    fn clean(merged: Graph, conflicts: BTreeSet<Slot>) -> Self {
        Self { merged, conflicts, unparseable_lines: 0, available: true }
    }
}

/// How a graph is written to disk for a line-merge arm.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    /// Subject-grouped Turtle, independently re-serialised per side — two
    /// copies that diverged were also written out by different tools or hands.
    TurtleReserialized,
    /// Subject-grouped Turtle in one stable order for all three inputs — the
    /// BEST case for a line merge, and the one a reviewer will ask about: two
    /// people editing the same committed `.ttl` in place.
    TurtleStable,
    /// The sorted triple set, one triple per line — a share bundle's
    /// `export.nt`.
    Canonical,
}

/// The strategies, in the order the paper's table reports them.
pub const ARMS: &[&str] = &[
    "git-turtle-reserialized",
    "git-turtle-stable",
    "git-canonical",
    "union",
    "lww-theirs",
    "triple-3way",
    "context-merge",
    "shape-aware",
];

/// Run one arm by name.
#[must_use]
pub fn run(arm: &str, base: &Graph, ours: &Graph, theirs: &Graph) -> Outcome {
    match arm {
        "git-turtle-reserialized" => {
            git_line_merge(base, ours, theirs, Form::TurtleReserialized)
        }
        "git-turtle-stable" => git_line_merge(base, ours, theirs, Form::TurtleStable),
        "git-canonical" => git_line_merge(base, ours, theirs, Form::Canonical),
        "union" => union(ours, theirs),
        "lww-theirs" => lww(base, ours, theirs),
        "triple-3way" => triple_3way(base, ours, theirs),
        "context-merge" => context_merge(base, ours, theirs),
        "shape-aware" => shape_aware(base, ours, theirs),
        other => panic!("unknown arm: {other}"),
    }
}

/// Rebuild a graph from per-slot value sets, holding conflicted slots at base.
fn assemble(
    base: &BTreeMap<Slot, BTreeSet<Term>>,
    resolved: &BTreeMap<Slot, BTreeSet<Term>>,
    conflicts: &BTreeSet<Slot>,
) -> Graph {
    let mut g = Graph::new();
    let mut slots: BTreeSet<&Slot> = resolved.keys().collect();
    slots.extend(conflicts.iter());
    for slot in slots {
        let values = if conflicts.contains(slot) {
            base.get(slot)
        } else {
            resolved.get(slot)
        };
        for v in values.into_iter().flatten() {
            g.insert(Triple::new(&slot.0, &slot.1, v));
        }
    }
    g
}

/// Every slot mentioned by any of the three graphs.
fn all_slots(
    b: &BTreeMap<Slot, BTreeSet<Term>>,
    o: &BTreeMap<Slot, BTreeSet<Term>>,
    t: &BTreeMap<Slot, BTreeSet<Term>>,
) -> BTreeSet<Slot> {
    let mut s: BTreeSet<Slot> = BTreeSet::new();
    s.extend(b.keys().cloned());
    s.extend(o.keys().cloned());
    s.extend(t.keys().cloned());
    s
}

/// Naive set union. Never asks anyone anything; keeps every value both sides
/// ever held, including the ones each side deleted.
fn union(ours: &Graph, theirs: &Graph) -> Outcome {
    let mut g = ours.clone();
    g.extend(theirs.iter().cloned());
    Outcome::clean(g, BTreeSet::new())
}

/// Last-writer-wins at slot granularity, with `theirs` as the later writer.
/// A slot theirs touched is taken wholesale from theirs; otherwise ours.
fn lww(base: &Graph, ours: &Graph, theirs: &Graph) -> Outcome {
    let (b, o, t) = (by_slot(base), by_slot(ours), by_slot(theirs));
    let empty = BTreeSet::new();
    let mut resolved = BTreeMap::new();
    for slot in all_slots(&b, &o, &t) {
        let bv = b.get(&slot).unwrap_or(&empty);
        let tv = t.get(&slot).unwrap_or(&empty);
        let ov = o.get(&slot).unwrap_or(&empty);
        let winner = if tv == bv { ov } else { tv };
        resolved.insert(slot, winner.clone());
    }
    Outcome::clean(assemble(&b, &resolved, &BTreeSet::new()), BTreeSet::new())
}

/// Triple-set three-way merge with no schema knowledge — the established
/// operator (Quit Store's default strategy). Set algebra is always defined, so
/// it never reports a conflict and never asks a human anything.
fn triple_3way(base: &Graph, ours: &Graph, theirs: &Graph) -> Outcome {
    let (b, o, t) = (by_slot(base), by_slot(ours), by_slot(theirs));
    let empty = BTreeSet::new();
    let mut resolved = BTreeMap::new();
    for slot in all_slots(&b, &o, &t) {
        let merged = set_three_way(
            b.get(&slot).unwrap_or(&empty),
            o.get(&slot).unwrap_or(&empty),
            t.get(&slot).unwrap_or(&empty),
        );
        resolved.insert(slot, merged);
    }
    Outcome::clean(assemble(&b, &resolved, &BTreeSet::new()), BTreeSet::new())
}

/// Node-overlap context merge — the Quit Context Merge heuristic, and the
/// nearest neighbour named in the novelty ruling. Both sides' changes are
/// projected onto the nodes they touch (subject or object); a node touched by
/// both sides makes every slot on that node a conflict.
fn context_merge(base: &Graph, ours: &Graph, theirs: &Graph) -> Outcome {
    let touched = |side: &Graph| -> BTreeSet<String> {
        let mut nodes = BTreeSet::new();
        for t in side.symmetric_difference(base) {
            nodes.insert(t.s.clone());
            if let Some(iri) = t.o.strip_prefix('<').and_then(|v| v.strip_suffix('>')) {
                nodes.insert(iri.to_string());
            }
        }
        nodes
    };
    let contended: BTreeSet<String> =
        touched(ours).intersection(&touched(theirs)).cloned().collect();

    let (b, o, t) = (by_slot(base), by_slot(ours), by_slot(theirs));
    let empty = BTreeSet::new();
    let mut resolved = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    for slot in all_slots(&b, &o, &t) {
        let bv = b.get(&slot).unwrap_or(&empty);
        let ov = o.get(&slot).unwrap_or(&empty);
        let tv = t.get(&slot).unwrap_or(&empty);
        if contended.contains(&slot.0) && (ov != bv || tv != bv) {
            conflicts.insert(slot.clone());
        }
        resolved.insert(slot, set_three_way(bv, ov, tv));
    }
    Outcome::clean(assemble(&b, &resolved, &conflicts), conflicts)
}

/// The shape-aware operator: set-algebraic three-way merge, with the shapes
/// graph deciding which slots the algebra is ALLOWED to settle.
///
/// It reads only the three graphs and the shapes. It has no access to the edit
/// scripts, and it does not know the oracle exists — but on the synthetic arm
/// its rules for the three triple-visible classes coincide with the oracle's by
/// construction, so its score there is a property of the design and not
/// evidence about it. What the synthetic arm does measure honestly is the
/// alias class it cannot see and the cost the other arms pay. See
/// `benchmark/mergebench/BUILD_REPORT.md` §3.
fn shape_aware(base: &Graph, ours: &Graph, theirs: &Graph) -> Outcome {
    let (b, o, t) = (by_slot(base), by_slot(ours), by_slot(theirs));
    let empty = BTreeSet::new();
    let mut resolved = BTreeMap::new();
    let mut conflicts = BTreeSet::new();

    for slot in all_slots(&b, &o, &t) {
        let bv = b.get(&slot).unwrap_or(&empty);
        let ov = o.get(&slot).unwrap_or(&empty);
        let tv = t.get(&slot).unwrap_or(&empty);
        let merged = set_three_way(bv, ov, tv);

        if let Some(bound) = shapes::max_count(&slot.1)
            && ov != tv
        {
            if merged.len() > bound {
                conflicts.insert(slot.clone());
            } else if bound == 1 && !bv.is_empty() {
                let ours_removed = ov.is_empty();
                let theirs_removed = tv.is_empty();
                let ours_changed = !ov.is_empty() && ov != bv;
                let theirs_changed = !tv.is_empty() && tv != bv;
                if (ours_removed && theirs_changed) || (theirs_removed && ours_changed) {
                    conflicts.insert(slot.clone());
                }
            }
        }
        resolved.insert(slot, merged);
    }
    Outcome::clean(assemble(&b, &resolved, &conflicts), conflicts)
}

/// Whether `git merge-file` can run here. An unavailable arm is reported as
/// unavailable; it is never reported as a zero.
#[must_use]
pub fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Line-based three-way merge by the real `git merge-file`, over either the
/// canonical sorted N-Triples form or subject-grouped Turtle.
///
/// This is git, not a reimplementation of git: a line-merge baseline that a
/// reviewer cannot check against the tool it claims to be is not a baseline.
fn git_line_merge(base: &Graph, ours: &Graph, theirs: &Graph, form: Form) -> Outcome {
    if !git_available() {
        return Outcome {
            merged: Graph::new(),
            conflicts: BTreeSet::new(),
            unparseable_lines: 0,
            available: false,
        };
    }
    let render = |g: &Graph, perm: u64| -> String {
        match form {
            Form::Canonical => model::to_canonical_nt(g),
            Form::TurtleStable => model::to_turtle(g, 0),
            Form::TurtleReserialized => model::to_turtle(g, perm),
        }
    };
    // The three forms are the ablation: `stable` isolates what line-merging
    // costs on RDF even when nothing re-orders; `reserialized` adds the
    // ordering churn that independent copies actually accumulate; `canonical`
    // removes ordering as a variable entirely. H3 is the gap between them.
    let dir = std::env::temp_dir().join(format!(
        "mergebench-{}-{}",
        std::process::id(),
        match form {
            Form::Canonical => "canonical",
            Form::TurtleStable => "stable",
            Form::TurtleReserialized => "reserialized",
        }
    ));
    let _ = std::fs::create_dir_all(&dir);
    let paths = ["base", "ours", "theirs"].map(|n| dir.join(n));
    let _ = std::fs::write(&paths[0], render(base, 0));
    let _ = std::fs::write(&paths[1], render(ours, 1));
    let _ = std::fs::write(&paths[2], render(theirs, 2));

    let out = Command::new("git")
        .args(["merge-file", "-p", "-q"])
        .arg(&paths[1])
        .arg(&paths[0])
        .arg(&paths[2])
        .output();
    let _ = std::fs::remove_dir_all(&dir);

    let Ok(out) = out else {
        return Outcome {
            merged: Graph::new(),
            conflicts: BTreeSet::new(),
            unparseable_lines: 0,
            available: false,
        };
    };
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if let Ok(dbg) = std::env::var("MERGEBENCH_DUMP") {
        let name = match form {
            Form::Canonical => "canonical",
            Form::TurtleStable => "stable",
            Form::TurtleReserialized => "reserialized",
        };
        let _ = std::fs::write(format!("{dbg}/merge-{name}.out"), &text);
    }
    parse_merge_output(&text, base, form == Form::Canonical)
}

/// Re-read `git merge-file` output in ONE sequential pass, tagging each
/// recovered triple with whether it sat inside a conflict hunk.
///
/// The pass is sequential on purpose. An earlier version split the clean
/// region from the hunks and parsed them separately, which stripped every
/// hunk's subject header (it lives in the clean text above the marker) and
/// reported 331 unparseable lines on an arm whose output git had written
/// perfectly well. That number measured the harness, not the baseline. Turtle
/// statement state crosses marker boundaries, so the reader has to as well.
///
/// A conflict hunk charges a human for every slot it mentions, and those slots
/// are held at base — the same accounting every other arm gets.
fn parse_merge_output(text: &str, base: &Graph, canonical: bool) -> Outcome {
    let mut clean = Graph::new();
    let mut hunk = Graph::new();
    let mut bad = 0usize;
    let mut in_hunk = false;
    let mut reader = TurtleReader::default();

    for line in text.lines() {
        if line.starts_with("<<<<<<<") {
            in_hunk = true;
            continue;
        }
        if line.starts_with(">>>>>>>") {
            in_hunk = false;
            continue;
        }
        // `=======` and `|||||||` separate the sides within a hunk; the
        // statement context carries across them, which is exactly why the
        // reader is not reset here.
        if line.starts_with("=======") || line.starts_with("|||||||") {
            continue;
        }
        let parsed = if canonical {
            read_nt_line(line)
        } else {
            reader.read_line(line)
        };
        match parsed {
            LineResult::Triple(t) => {
                if in_hunk { hunk.insert(t) } else { clean.insert(t) };
            }
            LineResult::Skip => {}
            LineResult::Bad => bad += 1,
        }
    }

    let conflicts: BTreeSet<Slot> = hunk.iter().map(Triple::slot).collect();
    let b = by_slot(base);
    let clean_by_slot = by_slot(&clean);
    let merged = assemble(&b, &clean_by_slot, &conflicts);
    Outcome { merged, conflicts, unparseable_lines: bad, available: true }
}

/// What one line of merge output yielded.
enum LineResult {
    /// A well-formed triple.
    Triple(Triple),
    /// Blank, comment, prefix declaration, or a subject header.
    Skip,
    /// Not readable as RDF. This count is a reported result: it is the rate at
    /// which line-merging RDF produces output that is not RDF.
    Bad,
}

fn read_nt_line(line: &str) -> LineResult {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return LineResult::Skip;
    }
    Triple::from_nt(line).map_or(LineResult::Bad, LineResult::Triple)
}

/// A deliberately narrow, STATEFUL Turtle reader for merge output.
///
/// It understands exactly the shape [`model::to_turtle`] writes, and it holds
/// the current subject across lines because a `;` list is a multi-line
/// statement. Anything it cannot read — a predicate-object pair whose subject
/// header was never seen, a token that is neither a prefixed name, an IRI, nor
/// a quoted literal — is counted, never guessed at.
#[derive(Default)]
struct TurtleReader {
    subject: Option<String>,
}

impl TurtleReader {
    fn read_line(&mut self, raw: &str) -> LineResult {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('@') || line.starts_with('#') {
            return LineResult::Skip;
        }
        let (body, terminator) = match line.strip_suffix(';') {
            Some(b) => (b.trim_end(), Some(';')),
            None => match line.strip_suffix('.') {
                Some(b) => (b.trim_end(), Some('.')),
                None => (line, None),
            },
        };
        // A bare token with no terminator is a subject header.
        if terminator.is_none() {
            return match expand_term(body) {
                Some(s) => {
                    self.subject = Some(s);
                    LineResult::Skip
                }
                None => LineResult::Bad,
            };
        }
        let Some(subject) = self.subject.clone() else {
            // A predicate-object pair with no subject in scope: the statement
            // was cut in half. Exactly the failure this count exists for.
            return LineResult::Bad;
        };
        let mut parts = body.splitn(2, char::is_whitespace);
        let (Some(p), Some(o)) = (parts.next(), parts.next()) else {
            return LineResult::Bad;
        };
        let result = match (expand_term(p), object_term(o)) {
            (Some(p), Some(o)) => LineResult::Triple(Triple::new(&subject, p, o)),
            _ => LineResult::Bad,
        };
        if terminator == Some('.') {
            self.subject = None;
        }
        result
    }
}

/// Expand a prefixed name or an angle-bracketed IRI to a bare IRI.
fn expand_term(tok: &str) -> Option<String> {
    let tok = tok.trim();
    if let Some(local) = tok.strip_prefix("bench:") {
        return Some(format!("{}{local}", shapes::NS));
    }
    if let Some(local) = tok.strip_prefix("rdf:") {
        return Some(format!("http://www.w3.org/1999/02/22-rdf-syntax-ns#{local}"));
    }
    tok.strip_prefix('<').and_then(|v| v.strip_suffix('>')).map(str::to_string)
}

/// Read an object position: a quoted literal, or an IRI re-bracketed into the
/// N-Triples object form the model uses.
fn object_term(tok: &str) -> Option<String> {
    let tok = tok.trim();
    if tok.starts_with('"') {
        return (tok.len() > 1 && tok.ends_with('"')).then(|| tok.to_string());
    }
    expand_term(tok).map(|iri| format!("<{iri}>"))
}

/// The reference result an operator with a perfect oracle would land: the
/// set-algebraic merge everywhere the oracle says no decision is owed, and the
/// base value held wherever it says one is.
///
/// This is the yardstick for `triples_lost` / `triples_spurious`. It is NOT a
/// strategy: it consumes the oracle, which no operator has.
#[must_use]
pub fn ideal(base: &Graph, ours: &Graph, theirs: &Graph, truth: &BTreeSet<Slot>) -> Graph {
    let (b, o, t) = (by_slot(base), by_slot(ours), by_slot(theirs));
    let empty = BTreeSet::new();
    let mut resolved = BTreeMap::new();
    for slot in all_slots(&b, &o, &t) {
        let merged = set_three_way(
            b.get(&slot).unwrap_or(&empty),
            o.get(&slot).unwrap_or(&empty),
            t.get(&slot).unwrap_or(&empty),
        );
        resolved.insert(slot, merged);
    }
    assemble(&b, &resolved, truth)
}
