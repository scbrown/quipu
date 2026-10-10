//! Bounded per-thread SPARQL regular-expression compilation.

use crate::error::{Error, Result};

/// Compiled patterns, per thread. A FILTER evaluates per row, so without this
/// `REGEX(?l, "x")` recompiled the same pattern for every one of ~1.1M labels:
/// ~11 µs/row, 13 s on the production label scan (aegis-tl2q4j). `Regex` clones
/// share one compiled program. Bounded: cleared when it reaches the cap.
const REGEX_CACHE_CAP: usize = 64;

thread_local! {
    static REGEX_CACHE: std::cell::RefCell<std::collections::HashMap<(String, String), regex::Regex>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Compile a SPARQL REGEX pattern + flag string into a `regex::Regex`, cached.
pub(in crate::sparql) fn build_regex(pattern: &str, flags: &str) -> Result<regex::Regex> {
    let key = (pattern.to_string(), flags.to_string());
    if let Some(re) = REGEX_CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Ok(re);
    }
    let re = compile_regex(pattern, flags)?;
    REGEX_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= REGEX_CACHE_CAP {
            c.clear();
        }
        c.insert(key, re.clone());
    });
    Ok(re)
}

fn compile_regex(pattern: &str, flags: &str) -> Result<regex::Regex> {
    let mut inline = String::new();
    for f in flags.chars() {
        match f {
            'i' | 's' | 'm' | 'x' => inline.push(f),
            other => {
                return Err(Error::InvalidValue(format!(
                    "unsupported REGEX flag: {other:?}"
                )));
            }
        }
    }
    let full = if inline.is_empty() {
        pattern.to_string()
    } else {
        format!("(?{inline}){pattern}")
    };
    regex::Regex::new(&full)
        .map_err(|e| Error::InvalidValue(format!("invalid REGEX pattern {pattern:?}: {e}")))
}
