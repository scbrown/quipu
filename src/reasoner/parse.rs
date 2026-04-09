//! Turtle → rule AST parser.
//!
//! Rules are stored as Turtle triples in the shapes table, alongside SHACL
//! shapes. A rule resource is any subject with `a rule:Rule`, carrying the
//! required `rule:id`, `rule:head`, and `rule:body` literal properties.
//!
//! The Turtle layer is handled by [`oxrdfio::RdfParser`]. The body and head
//! strings themselves are a tiny Datalog-surface syntax parsed by hand here
//! — keeping the dependency footprint at zero for this layer and making
//! error messages specific to the rule DSL instead of RDF.

use std::collections::BTreeMap;

use oxrdf::{NamedOrBlankNode, Term as OxTerm};
use oxrdfio::{RdfFormat, RdfParser};

use super::ast::{Atom, BodyAtom, Rule, Term};
use super::{
    RULE_BODY, RULE_DEFAULT_PREFIX, RULE_HEAD, RULE_ID, RULE_PREFIX, RULE_SET_TYPE, RULE_TYPE,
    ReasonerError, Result,
};
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};

/// A parsed ruleset ready for stratification and evaluation.
#[derive(Debug, Clone)]
pub struct RuleSet {
    /// Parsed rules, in the order they appeared in the Turtle source.
    pub rules: Vec<Rule>,
    /// Default IRI prefix used to resolve unqualified predicate names.
    pub default_prefix: String,
}

impl RuleSet {
    /// Empty ruleset with the given default prefix.
    pub fn empty(default_prefix: impl Into<String>) -> Self {
        Self {
            rules: Vec::new(),
            default_prefix: default_prefix.into(),
        }
    }

    /// Number of rules.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// True when no rules are loaded.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// Parse a Turtle source into a `RuleSet`.
///
/// Unqualified predicate names (e.g. `affects` instead of
/// `<http://aegis.gastown.local/ontology/affects>`) are expanded by
/// prepending a prefix chosen in this order:
///
/// 1. A per-rule `rule:prefix` property, if present.
/// 2. A `rule:defaultPrefix` attached to any `rule:RuleSet` subject.
/// 3. The `fallback_prefix` argument (typically
///    [`DEFAULT_BASE_NS`](crate::namespace::DEFAULT_BASE_NS)).
pub fn parse_rules(turtle: &str, fallback_prefix: Option<&str>) -> Result<RuleSet> {
    // Collect all triples into a subject-indexed property map so we can
    // assemble each rule without assuming a specific triple order.
    let mut by_subject: BTreeMap<String, Properties> = BTreeMap::new();

    let parser = RdfParser::from_format(RdfFormat::Turtle);
    for result in parser.for_reader(turtle.as_bytes()) {
        let quad = result.map_err(|e| ReasonerError::Turtle(format!("{e}")))?;
        let triple = oxrdf::Triple::from(quad);

        let subject_key = subject_key(&triple.subject);
        let predicate = triple.predicate.as_str().to_string();
        let entry = by_subject.entry(subject_key).or_default();
        entry.push(predicate, triple.object);
    }

    // First pass: find a ruleset-level default prefix, if any.
    let mut ruleset_default: Option<String> = None;
    for props in by_subject.values() {
        if props.has_type(RULE_SET_TYPE)
            && let Some(p) = props.first_str(RULE_DEFAULT_PREFIX)
        {
            ruleset_default = Some(p.to_string());
            break;
        }
    }
    let default_prefix = ruleset_default
        .or_else(|| fallback_prefix.map(str::to_string))
        .unwrap_or_else(|| DEFAULT_BASE_NS.to_string());

    // Second pass: materialise rule resources.
    let mut rules = Vec::new();
    for (subject_iri, props) in &by_subject {
        if !props.has_type(RULE_TYPE) {
            continue;
        }

        let id = props
            .first_str(RULE_ID)
            .map_or_else(|| subject_iri.clone(), str::to_string);

        let head_src =
            props
                .first_str(RULE_HEAD)
                .ok_or_else(|| ReasonerError::MissingProperty {
                    id: id.clone(),
                    property: "rule:head",
                })?;
        let body_src =
            props
                .first_str(RULE_BODY)
                .ok_or_else(|| ReasonerError::MissingProperty {
                    id: id.clone(),
                    property: "rule:body",
                })?;
        let rule_prefix = props
            .first_str(RULE_PREFIX)
            .map_or_else(|| default_prefix.clone(), str::to_string);

        let head = parse_atom(head_src, &rule_prefix).map_err(|m| ReasonerError::BadSyntax {
            id: id.clone(),
            location: "rule:head",
            message: m,
        })?;
        let body = parse_body(body_src, &rule_prefix).map_err(|m| ReasonerError::BadSyntax {
            id: id.clone(),
            location: "rule:body",
            message: m,
        })?;

        let rule = Rule { id, head, body };

        // Range-restriction check: every head variable must appear in a
        // positive body atom. This is the classic safety condition for
        // Datalog and rules out free variables in the head.
        let body_vars = rule.body_vars();
        for v in rule.head_vars() {
            if !body_vars.contains(&v) {
                return Err(ReasonerError::UnboundHeadVariable {
                    id: rule.id.clone(),
                    variable: v.to_string(),
                });
            }
        }

        rules.push(rule);
    }

    Ok(RuleSet {
        rules,
        default_prefix,
    })
}

// ── Turtle subject/property bookkeeping ────────────────────────

fn subject_key(subject: &NamedOrBlankNode) -> String {
    match subject {
        NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

#[derive(Debug, Default)]
struct Properties {
    pairs: Vec<(String, OxTerm)>,
}

impl Properties {
    fn push(&mut self, predicate: String, object: OxTerm) {
        self.pairs.push((predicate, object));
    }

    fn has_type(&self, type_iri: &str) -> bool {
        for (p, o) in &self.pairs {
            if p == RDF_TYPE
                && let OxTerm::NamedNode(n) = o
                && n.as_str() == type_iri
            {
                return true;
            }
        }
        false
    }

    /// First string-valued object for the given property IRI.
    fn first_str(&self, predicate: &str) -> Option<&str> {
        for (p, o) in &self.pairs {
            if p == predicate
                && let OxTerm::Literal(lit) = o
            {
                return Some(lit.value());
            }
        }
        None
    }
}

// ── Tiny Datalog-surface parser ────────────────────────────────

/// Parse a comma-separated list of body atoms.
pub(crate) fn parse_body(src: &str, prefix: &str) -> std::result::Result<Vec<BodyAtom>, String> {
    let chunks = split_top_level_commas(src);
    if chunks.is_empty() {
        return Err("empty body".into());
    }
    let mut out = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        out.push(parse_body_atom(chunk.trim(), prefix)?);
    }
    Ok(out)
}

/// Parse a single `[not] pred(args...)` literal.
fn parse_body_atom(src: &str, prefix: &str) -> std::result::Result<BodyAtom, String> {
    let src = src.trim();
    if src.is_empty() {
        return Err("empty body atom".into());
    }
    // `not` must be followed by whitespace to distinguish from an identifier
    // that happens to start with "not" (e.g. `notated`).
    let (negated, rest) = if let Some(r) = src.strip_prefix("not ") {
        (true, r.trim_start())
    } else if let Some(r) = src.strip_prefix("not\t") {
        (true, r.trim_start())
    } else {
        (false, src)
    };
    let atom = parse_atom(rest, prefix)?;
    Ok(if negated {
        BodyAtom::Negative(atom)
    } else {
        BodyAtom::Positive(atom)
    })
}

/// Parse a single `pred(args...)` application.
pub(crate) fn parse_atom(src: &str, prefix: &str) -> std::result::Result<Atom, String> {
    let src = src.trim();
    let open = src
        .find('(')
        .ok_or_else(|| format!("expected `(` in atom {src:?}"))?;
    if !src.ends_with(')') {
        return Err(format!("expected trailing `)` in atom {src:?}"));
    }
    let name = src[..open].trim();
    if name.is_empty() {
        return Err(format!("missing predicate name in atom {src:?}"));
    }
    let args_str = &src[open + 1..src.len() - 1];
    let arg_tokens = split_top_level_commas(args_str);
    let mut args = Vec::with_capacity(arg_tokens.len());
    for token in arg_tokens {
        args.push(parse_term(token.trim(), prefix)?);
    }
    Ok(Atom {
        predicate: resolve_iri(name, prefix),
        args,
    })
}

pub(crate) fn parse_term(token: &str, prefix: &str) -> std::result::Result<Term, String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("empty argument".into());
    }
    if let Some(name) = token.strip_prefix('?') {
        if name.is_empty() {
            return Err("variable name missing after `?`".into());
        }
        return Ok(Term::Var(name.to_string()));
    }
    if token.starts_with('<') && token.ends_with('>') && token.len() >= 2 {
        return Ok(Term::Iri(token[1..token.len() - 1].to_string()));
    }
    if token.starts_with('"') && token.ends_with('"') && token.len() >= 2 {
        return Ok(Term::Str(token[1..token.len() - 1].to_string()));
    }
    // Bare identifier → treat as an IRI under the default prefix.
    Ok(Term::Iri(format!("{prefix}{token}")))
}

/// Expand a bare predicate name against a prefix, or leave an explicit IRI
/// alone.
fn resolve_iri(name: &str, prefix: &str) -> String {
    let name = name.trim();
    if name.starts_with('<') && name.ends_with('>') && name.len() >= 2 {
        name[1..name.len() - 1].to_string()
    } else {
        format!("{prefix}{name}")
    }
}

/// Split a string on commas that are not nested inside parentheses or
/// double-quoted strings. Empty chunks are dropped.
pub(crate) fn split_top_level_commas(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0_i32;
    let mut in_quote = false;
    for ch in src.chars() {
        if in_quote {
            current.push(ch);
            if ch == '"' {
                in_quote = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_quote = true;
                current.push(ch);
            }
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    out
}
