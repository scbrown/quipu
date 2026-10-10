//! One conservative bound on SPARQL structure, and one place every request is
//! parsed (aegis-rq1afp, aegis-xcvb5z).
//!
//! The parser and the algebra walks behind it recurse with the request's
//! STRUCTURE, not its size: nesting deepens them, and so do chains (operators,
//! path alternatives, `UNION`s, sibling filters). On a pool thread's default
//! stack a request of a few kilobytes overflowed and ABORTED the process, which
//! is an outage for every caller, not a refused request.
//!
//! [`structural_cost`] OVER-counts by design. It skips only what cannot recurse
//! and can be recognised soundly (string literals, names, datatype tags); every
//! bracket counts wherever it appears outside a string, every operator
//! character counts, and so do the keywords that open a nested algebra node.
//! Over-refusing a pathological literal is acceptable; under-counting a request
//! that reaches the parser is not. [`parse_query`] and [`parse_update`] apply
//! the bound and then parse on a dedicated thread with [`DEEP_STACK_BYTES`], so
//! the cap has a margin even in a debug build.

use crate::error::{Error, Result};

/// The most structural tokens one request may carry. Sized to the DEBUG profile,
/// whose frames are ~25x release's: on [`DEEP_STACK_BYTES`] every shape in the
/// regression matrix parses at this cost in a debug build.
pub const STRUCTURE_LIMIT: usize = 4096;

/// Stack for parsing and evaluating caller-supplied SPARQL. Virtual memory,
/// committed only as deep as a request actually recurses.
pub const DEEP_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Beyond this length an IRI-shaped region is counted as if it were an
/// expression: a long `<...>` could be parsed as a relational expression
/// carrying an operator chain.
const LONG_IRI: usize = 512;

const KEYWORDS: [&str; 6] = ["FILTER", "UNION", "MINUS", "OPTIONAL", "EXISTS", "NOT"];

/// Operator characters that can chain or nest an expression or a path.
fn is_operator(c: u8) -> bool {
    matches!(
        c,
        b'|' | b'&' | b'+' | b'-' | b'*' | b'/' | b'!' | b'^' | b'=' | b'<' | b'>'
    )
}

/// An upper bound on how deep `text` can drive the parser. See the module docs.
#[must_use]
pub fn structural_cost(text: &str) -> usize {
    let b = text.as_bytes();
    let mut cost = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'') => {
                i = skip_string(b, i, q);
                // A datatype tag is not the inverse-path operator, and a
                // language tag's '-' is not subtraction.
                if b.get(i) == Some(&b'^') && b.get(i + 1) == Some(&b'^') {
                    i += 2;
                } else if b.get(i) == Some(&b'@') {
                    i += 1;
                    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
                        i += 1;
                    }
                }
            }
            b'<' => {
                if let Some(end) = iri_end(b, i) {
                    let body = &b[i + 1..end];
                    let long = body.len() > LONG_IRI;
                    cost += body
                        .iter()
                        .filter(|&&c| {
                            matches!(c, b'(' | b'[' | b'{')
                                || matches!(c, b'+' | b'*' | b'!' | b'^')
                                || (long && is_operator(c))
                        })
                        .count();
                    i = end + 1;
                } else {
                    cost += 1;
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => {
                cost += 1;
                i += 1;
            }
            c if is_operator(c) => {
                cost += 1;
                i += 1;
            }
            // Variables: names only, never containing an operator character.
            b'?' | b'$' => {
                i += 1;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80)
                {
                    i += 1;
                }
            }
            // A prefixed name's local part, which may legally contain '-' and '.'.
            b':' => {
                i += 1;
                while i < b.len() && is_local(b[i]) {
                    i += 1;
                }
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] >= 0x80)
                {
                    i += 1;
                }
                let word = &text[start..i];
                if KEYWORDS.iter().any(|k| word.eq_ignore_ascii_case(k)) {
                    cost += 1;
                }
            }
            _ => i += 1,
        }
    }
    cost
}

/// Refuse a request whose structure exceeds [`STRUCTURE_LIMIT`].
pub fn check(text: &str) -> Result<()> {
    let cost = structural_cost(text);
    if cost > STRUCTURE_LIMIT {
        return Err(Error::InvalidValue(format!(
            "SPARQL request is too deeply nested or too long a chain: structural cost \
             {cost} exceeds the limit of {STRUCTURE_LIMIT}. Split it into smaller requests."
        )));
    }
    Ok(())
}

/// Run `f` on a thread with [`DEEP_STACK_BYTES`] of stack. Scoped, so `f` may
/// borrow. A panic in `f` is re-raised on the caller. On wasm32 there are no
/// threads; `f` runs in place and the structural bound is the only defence.
#[cfg(target_arch = "wasm32")]
pub fn on_deep_stack<T: Send>(f: impl FnOnce() -> T + Send) -> Result<T> {
    Ok(f())
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// Set on a thread started by [`on_deep_stack`], so nested calls (a server
    /// request already on a deep stack, then parsing) run in place instead of
    /// paying for a second thread.
    static ON_DEEP_STACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` on a thread with [`DEEP_STACK_BYTES`] of stack. Scoped, so `f` may
/// borrow. A panic in `f` is re-raised on the caller. Already on such a
/// thread, `f` runs in place.
#[cfg(not(target_arch = "wasm32"))]
pub fn on_deep_stack<T: Send>(f: impl FnOnce() -> T + Send) -> Result<T> {
    if ON_DEEP_STACK.with(std::cell::Cell::get) {
        return Ok(f());
    }
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .name("quipu-parse".into())
            .stack_size(DEEP_STACK_BYTES)
            .spawn_scoped(scope, move || {
                ON_DEEP_STACK.with(|deep| deep.set(true));
                f()
            })
            .map_err(|e| Error::InvalidValue(format!("could not start parse thread: {e}")))?;
        Ok(handle
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)))
    })
}

/// Bound, then parse a query on a deep stack. The outer `Result` is the bound;
/// the inner one is the parser's own verdict.
pub fn parse_query(
    parser: spargebra::SparqlParser,
    text: &str,
) -> Result<std::result::Result<spargebra::Query, spargebra::SparqlSyntaxError>> {
    check(text)?;
    on_deep_stack(move || parser.parse_query(text))
}

/// [`parse_query`] for a SPARQL Update.
pub fn parse_update(
    parser: spargebra::SparqlParser,
    text: &str,
) -> Result<std::result::Result<spargebra::Update, spargebra::SparqlSyntaxError>> {
    check(text)?;
    on_deep_stack(move || parser.parse_update(text))
}

fn is_local(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':' | b'%') || c >= 0x80
}

/// Skip a string literal starting at `i`, short or long, with backslash
/// escapes. Quote delimiters are unambiguous outside an IRI, and IRI-shaped
/// regions are consumed before a quote inside them can be read.
fn skip_string(b: &[u8], i: usize, q: u8) -> usize {
    let long = b.len() >= i + 3 && b[i + 1] == q && b[i + 2] == q;
    let mut j = if long { i + 3 } else { i + 1 };
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == q => {
                if !long {
                    return j + 1;
                }
                if b.len() >= j + 3 && b[j + 1] == q && b[j + 2] == q {
                    return j + 3;
                }
                j += 1;
            }
            b'\n' | b'\r' if !long => return j + 1,
            _ => j += 1,
        }
    }
    b.len()
}

/// The index of the `>` closing an IRIREF-shaped region that starts at `i`,
/// using SPARQL's IRIREF character set. `None` means `<` is an operator here.
fn iri_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'>' => return Some(j),
            b'<' | b'"' | b'{' | b'}' | b'|' | b'^' | b'`' | b'\\' => return None,
            c if c <= b' ' => return None,
            _ => j += 1,
        }
    }
    None
}

#[cfg(test)]
#[path = "sparql_structure_tests.rs"]
mod sparql_structure_tests;
