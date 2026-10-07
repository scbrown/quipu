//! A pre-parse bound on how deep a SPARQL request can drive the parser.
//!
//! spargebra's parser and the algebra walks behind it are recursive, and their
//! depth follows the request's STRUCTURE, not its byte size. Nesting deepens
//! them, but so do CHAINS: a flat run of `UNION`s or sibling `FILTER`s folds
//! into a left-deep algebra tree. Measured on 674f3700 with the default 2 MiB
//! blocking stack (aegis-rq1afp): /update aborted the WHOLE PROCESS with "stack
//! overflow" at 500 nested `FILTER NOT EXISTS` (15 KB), 2,000 flat `UNION`s,
//! 5,000 sibling `FILTER NOT EXISTS` and 5,000 nested parentheses. A crash is
//! not a refusal: the store goes offline for every caller.
//!
//! [`structural_cost`] counts every token that can add a level, ignoring string
//! literals, IRIs and comments. It is an upper bound on depth, so a request
//! within [`STRUCTURE_LIMIT`] can be parsed on a stack sized for it.

use crate::error::{Error, Result};

/// The most structural tokens one request may carry. Callers parse on a stack
/// sized so that this many tokens, in every shape measured, still fit: on
/// 256 MiB the DEBUG build overflowed between 2,600 and 2,729 nested
/// `FILTER NOT EXISTS` (3 tokens per level), so 4,096 tokens (~1,365 levels) is
/// ~1.9x inside the worst profile and far inside release, which held 2,729
/// levels on 64 MiB. A sibling batch of ~1,300 `FILTER NOT EXISTS` guards fits.
pub const STRUCTURE_LIMIT: usize = 4096;

const KEYWORDS: [&str; 5] = ["FILTER", "UNION", "MINUS", "OPTIONAL", "EXISTS"];

/// Count `{`, `(` and the keywords that open a nested algebra node, outside
/// string literals, IRIs and comments.
#[must_use]
pub fn structural_cost(text: &str) -> usize {
    let b = text.as_bytes();
    let mut cost = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            q @ (b'"' | b'\'') => i = skip_string(b, i, q),
            b'<' => i = skip_iri(b, i),
            b'{' | b'(' => {
                cost += 1;
                i += 1;
            }
            // Variables and prefixed-name locals are names, never keywords.
            b'?' | b'$' | b':' => {
                i += 1;
                while i < b.len() && is_name(b[i]) {
                    i += 1;
                }
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < b.len() && is_name(b[i]) {
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

/// Refuse a request whose structure exceeds [`STRUCTURE_LIMIT`], before any
/// parser sees it.
pub fn check(text: &str) -> Result<()> {
    let cost = structural_cost(text);
    if cost > STRUCTURE_LIMIT {
        return Err(Error::InvalidValue(format!(
            "SPARQL request nests too deeply: {cost} structural tokens ({{, (, FILTER, \
             UNION, MINUS, OPTIONAL, EXISTS) exceeds the limit of {STRUCTURE_LIMIT}. \
             Split it into smaller requests (aegis-rq1afp)."
        )));
    }
    Ok(())
}

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.' || c >= 0x80
}

/// Skip a string literal starting at `i`, short or long (`'''`/`"""`), with
/// backslash escapes. An unterminated literal runs to the end.
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
            b'\n' if !long => return j + 1,
            _ => j += 1,
        }
    }
    b.len()
}

/// Skip an IRIREF at `i` if one starts there; otherwise `<` is the less-than
/// operator and only it is consumed.
fn skip_iri(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'>' => return j + 1,
            b'<' | b'"' | b'{' | b'}' | b'|' | b'^' | b'`' | b'\\' => return i + 1,
            c if c <= b' ' => return i + 1,
            _ => j += 1,
        }
    }
    i + 1
}

#[cfg(test)]
#[path = "sparql_structure_tests.rs"]
mod sparql_structure_tests;
