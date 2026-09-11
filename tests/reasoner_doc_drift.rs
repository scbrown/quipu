//! Keep the feature summaries aligned with the negation evaluator regressions.
//! The evaluator tests separately prove filtering, lower-stratum derived facts,
//! unsafe-variable rejection, and rejection of cycles through negation.

use std::path::Path;

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

#[test]
fn negation_support_is_documented_on_every_summary_surface() {
    let readme = read("README.md");
    for prefix in [
        "- **Datalog over EAVT**",
        "- **Datalog rule engine**",
        "| Datalog rule engine (datafrog)",
    ] {
        let line = readme
            .lines()
            .find(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing feature summary: {prefix}"));
        assert!(line.contains("stratified negation-as-failure"), "{line}");
        assert!(line.contains("negation cycles"), "{line}");
        assert!(!line.contains("not yet implemented"), "{line}");
        assert!(!line.contains("rejects `not` rules"), "{line}");
    }

    let ast = read("src/reasoner/ast.rs");
    let body_atom_docs = ast
        .split("/// A body literal:")
        .nth(1)
        .expect("BodyAtom documentation must exist")
        .split("impl BodyAtom")
        .next()
        .unwrap();
    assert!(body_atom_docs.contains("stratified negation-as-failure"));
    assert!(body_atom_docs.contains("bound by positive atoms"));
    assert!(!body_atom_docs.contains("evaluator rejects negated atoms"));
    assert!(!body_atom_docs.contains("reserved for future NAF support"));

    let reference = read("docs/book/src/reference/reasoner.md");
    assert!(reference.contains("stratified negation-as-failure"));
    assert!(reference.contains("unsafe negation is rejected"));
}
