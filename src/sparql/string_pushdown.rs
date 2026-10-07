//! Push string FILTERs into the SQL scan of the BGP they wrap (aegis-tl2q4j).
//!
//! `?s rdfs:label ?l FILTER(regex(str(?l), "x"))` used to materialize every
//! label as a Rust binding and only then run the filter: ~1.1M rows and ~2 s
//! on the production store, while `SQLite` itself scans those rows with a
//! substring test in ~0.2 s. Agents write these filters naturally, so each
//! one was a timeout.
//!
//! This module turns a top-level conjunct of a string test on one variable
//! into a NARROWING predicate the scan evaluates in SQL, through two scalar
//! functions registered on every connection:
//!
//! - `quipu_narrow_text(text, k)`: the variable is an IRI position
//!   (subject, predicate), tested against `terms.iri`;
//! - `quipu_narrow_value(v, k)`: the variable is the object, tested against
//!   the encoded value. An IRI object (`Ref`) is always KEPT: its string
//!   needs the store, which a scalar function cannot reach.
//!
//! **The real FILTER still runs on every surviving row**, so the narrowing
//! only has to keep every row the filter could accept; it never decides one.
//! It computes the same string the filter does with the same functions
//! ([`super::filter::lexical`], `to_lowercase`, the cached regex), so a row
//! the filter accepts is never dropped. Rows it keeps that the filter then
//! rejects (a type error, say) cost time, not correctness.
//!
//! Deliberately narrow, like [`super::filter_pushdown`]:
//! - only top-level conjuncts (`a && b`), never under `||` or `!`;
//! - `CONTAINS` / `STRSTARTS` / `STRENDS` / `REGEX` whose first argument is
//!   the variable, optionally under `STR`, `LCASE`, `UCASE`, and whose other
//!   arguments are literals;
//! - only when the filter wraps a plain BGP that mentions the variable, and
//!   the caller has not already bound it;
//! - on a composed store the IRI positions search `main.terms` AND every
//!   attachment's `terms`: term spaces make each layer's ids valid locally,
//!   and an alias id carries the same IRI as its canonical id, so the string
//!   the filter sees is the string the narrowing tests.

use std::cell::RefCell;
use std::sync::Arc;

use rusqlite::functions::FunctionFlags;
use spargebra::algebra::{Expression, Function, GraphPattern};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use super::filter::{build_regex, lexical, literal_to_value};
use super::pattern_util::Bindings;
use crate::types::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Test {
    Contains,
    Starts,
    Ends,
    Regex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fold {
    None,
    Lower,
    Upper,
}

/// One narrowing predicate on one variable.
#[derive(Debug, Clone)]
pub struct StringNarrow {
    /// The variable it narrows.
    pub var: String,
    test: Test,
    fold: Fold,
    needle: String,
    regex: Option<regex::Regex>,
}

impl StringNarrow {
    /// Could the filter accept a value whose string form is `s`?
    fn keeps_text(&self, s: &str) -> bool {
        let folded;
        let s = match self.fold {
            Fold::None => s,
            Fold::Lower => {
                folded = s.to_lowercase();
                &folded
            }
            Fold::Upper => {
                folded = s.to_uppercase();
                &folded
            }
        };
        match self.test {
            Test::Contains => s.contains(&self.needle),
            Test::Starts => s.starts_with(&self.needle),
            Test::Ends => s.ends_with(&self.needle),
            Test::Regex => self.regex.as_ref().is_none_or(|re| re.is_match(s)),
        }
    }

    /// [`Self::keeps_text`] for an encoded value; an undecodable value or an
    /// IRI is kept.
    fn keeps_encoded(&self, bytes: &[u8]) -> bool {
        match Value::from_bytes(bytes) {
            Ok(value) => lexical(&value).is_none_or(|s| self.keeps_text(&s)),
            Err(_) => true,
        }
    }
}

/// The narrowing predicates `expr` licenses for the BGP `inner`.
pub fn collect(expr: &Expression, inner: &GraphPattern, seed: &Bindings) -> Vec<StringNarrow> {
    let GraphPattern::Bgp { patterns } = inner else {
        return Vec::new();
    };
    let mut out = Vec::new();
    conjuncts(expr, &mut out);
    out.retain(|n| !seed.contains_key(&n.var) && occurs(patterns, &n.var));
    out
}

fn conjuncts(expr: &Expression, out: &mut Vec<StringNarrow>) {
    match expr {
        Expression::And(a, b) => {
            conjuncts(a, out);
            conjuncts(b, out);
        }
        Expression::FunctionCall(function, args) => {
            if let Some(n) = narrow(function, args) {
                out.push(n);
            }
        }
        _ => {}
    }
}

fn narrow(function: &Function, args: &[Expression]) -> Option<StringNarrow> {
    let test = match function {
        Function::Contains => Test::Contains,
        Function::StrStarts => Test::Starts,
        Function::StrEnds => Test::Ends,
        Function::Regex => Test::Regex,
        _ => return None,
    };
    let (var, fold) = subject_of(args.first()?)?;
    let needle = literal_text(args.get(1)?)?;
    let regex = if test == Test::Regex {
        let flags = match args.get(2) {
            Some(e) => literal_text(e)?,
            None => String::new(),
        };
        // An invalid pattern makes the filter an error; leave that to it.
        Some(build_regex(&needle, &flags).ok()?)
    } else {
        if args.len() != 2 {
            return None;
        }
        None
    };
    Some(StringNarrow {
        var,
        test,
        fold,
        needle,
        regex,
    })
}

/// `?v`, `STR(?v)`, and either under one `LCASE` / `UCASE`.
fn subject_of(expr: &Expression) -> Option<(String, Fold)> {
    let plain = |e: &Expression| -> Option<String> {
        match e {
            Expression::Variable(v) => Some(v.as_str().to_string()),
            Expression::FunctionCall(Function::Str, a) if a.len() == 1 => match &a[0] {
                Expression::Variable(v) => Some(v.as_str().to_string()),
                _ => None,
            },
            _ => None,
        }
    };
    match expr {
        Expression::FunctionCall(Function::LCase, a) if a.len() == 1 => {
            Some((plain(&a[0])?, Fold::Lower))
        }
        Expression::FunctionCall(Function::UCase, a) if a.len() == 1 => {
            Some((plain(&a[0])?, Fold::Upper))
        }
        other => Some((plain(other)?, Fold::None)),
    }
}

/// The string the filter itself derives from a literal argument.
fn literal_text(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Literal(lit) => lexical(&literal_to_value(lit)),
        _ => None,
    }
}

fn occurs(patterns: &[TriplePattern], var: &str) -> bool {
    patterns.iter().any(|tp| {
        matches!(&tp.subject, TermPattern::Variable(v) if v.as_str() == var)
            || matches!(&tp.predicate, NamedNodePattern::Variable(v) if v.as_str() == var)
            || matches!(&tp.object, TermPattern::Variable(v) if v.as_str() == var)
    })
}

thread_local! {
    /// The predicates the statement being stepped on this thread refers to by
    /// index. `SQLite` calls a scalar function on the thread stepping it.
    static ACTIVE: RefCell<Option<Arc<Vec<StringNarrow>>>> = const { RefCell::new(None) };
}

/// Makes `narrows` visible to the scalar functions until dropped.
pub struct Active(Option<Arc<Vec<StringNarrow>>>);

impl Active {
    /// Install `narrows` for this thread, remembering what was there.
    #[must_use]
    pub fn install(narrows: Arc<Vec<StringNarrow>>) -> Self {
        Self(ACTIVE.with(|a| a.borrow_mut().replace(narrows)))
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        let previous = self.0.take();
        ACTIVE.with(|a| *a.borrow_mut() = previous);
    }
}

/// Keep (1) unless the active predicate `k` rejects. No active set, or an
/// unknown index, keeps: the narrowing may only ever widen toward the filter.
fn keeps(k: i64, test: impl FnOnce(&StringNarrow) -> bool) -> bool {
    ACTIVE.with(|a| {
        a.borrow()
            .as_ref()
            .and_then(|set| set.get(usize::try_from(k).ok()?))
            .is_none_or(test)
    })
}

/// Register `quipu_narrow_text` and `quipu_narrow_value` on `conn`.
///
/// # Errors
/// [`rusqlite::Error`] if `SQLite` refuses the registration.
pub fn register(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_INNOCUOUS;
    conn.create_scalar_function("quipu_narrow_text", 2, flags, |ctx| {
        let k: i64 = ctx.get(1)?;
        let text = ctx.get_raw(0).as_str().ok().map(str::to_owned);
        Ok(keeps(k, |n| {
            text.as_deref().is_none_or(|s| n.keeps_text(s))
        }))
    })?;
    conn.create_scalar_function("quipu_narrow_value", 2, flags, |ctx| {
        let k: i64 = ctx.get(1)?;
        let bytes = ctx.get_raw(0).as_blob().ok().map(<[u8]>::to_vec);
        Ok(keeps(k, |n| {
            bytes.as_deref().is_none_or(|b| n.keeps_encoded(b))
        }))
    })?;
    // The term id of an IRI value, NULL for anything else, so an IRI object
    // can be narrowed through `terms` like a subject.
    conn.create_scalar_function(
        "quipu_ref_id",
        1,
        flags | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            Ok(ctx
                .get_raw(0)
                .as_blob()
                .ok()
                .and_then(|b| match Value::from_bytes(b) {
                    Ok(Value::Ref(id)) => Some(id),
                    _ => None,
                }))
        },
    )
}
