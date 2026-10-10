//! Offline legacy literal compatibility probe. Does not open a production store.
//! Run against the old build before changing storage; preserve its JSON output.
use oxrdfio::RdfFormat;
use quipu::{Datum, Op, Store, Value};

const INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";
const DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";
const TIME: &str = "2026-01-01T00:00:00Z";

fn typed(lexical: &str, datatype: &str) -> Value {
    Value::Typed {
        lexical: lexical.into(),
        datatype: datatype.into(),
    }
}

fn datum(e: i64, a: i64, value: Value, op: Op) -> Datum {
    Datum {
        entity: e,
        attribute: a,
        value,
        valid_from: TIME.into(),
        valid_to: None,
        op,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args()
        .nth(1)
        .ok_or("supply a NEW fixture directory")?;
    let seed_directory = std::env::args().nth(2);
    // Refuse to overwrite previous evidence or accidentally use an existing DB.
    std::fs::create_dir(&directory)?;
    let cases = vec![
        (
            "float_zero",
            Value::Float(0.0),
            typed("0", DOUBLE),
            typed("0.0", DOUBLE),
        ),
        (
            "float_negative_zero",
            Value::Float(-0.0),
            typed("-0", DOUBLE),
            typed("-0.0", DOUBLE),
        ),
        (
            "float_one",
            Value::Float(1.0),
            typed("1", DOUBLE),
            typed("1.0", DOUBLE),
        ),
        (
            "float_fraction",
            Value::Float(1.5),
            typed("1.5", DOUBLE),
            typed("1.50", DOUBLE),
        ),
        (
            "float_infinity",
            Value::Float(f64::INFINITY),
            typed("inf", DOUBLE),
            typed("INF", DOUBLE),
        ),
        (
            "float_negative_infinity",
            Value::Float(f64::NEG_INFINITY),
            typed("-inf", DOUBLE),
            typed("-INF", DOUBLE),
        ),
        (
            "float_nan",
            Value::Float(f64::NAN),
            typed("NaN", DOUBLE),
            typed("nan", DOUBLE),
        ),
        (
            "float_nan_payload",
            Value::Float(f64::from_bits(0x7ff8_0000_0000_0123)),
            typed("NaN", DOUBLE),
            typed("nan", DOUBLE),
        ),
        (
            "typed_integer",
            typed("1", INTEGER),
            Value::Int(1),
            typed("01", INTEGER),
        ),
        (
            "typed_boolean",
            typed("true", BOOLEAN),
            Value::Bool(true),
            typed("1", BOOLEAN),
        ),
    ];
    let mut report = Vec::new();
    for (name, legacy, equivalent, sibling) in cases {
        let path = std::path::Path::new(&directory).join(format!("{name}.db"));
        if let Some(seed) = &seed_directory {
            std::fs::copy(std::path::Path::new(seed).join(format!("{name}.db")), &path)?;
        }
        let mut store = Store::open(path.to_str().ok_or("non-UTF8 path")?)?;
        let e = store.intern("http://example.org/s")?;
        let a = store.intern("http://example.org/p")?;
        let old_term = quipu::rdf::value_to_term(&store, &legacy)?;
        let paired_term = quipu::rdf::value_to_term(&store, &equivalent)?;
        // Positive controls: the tested pair really is one RDF term, while the
        // sibling differs lexically (nonfinite siblings need not be valid).
        assert_eq!(old_term, paired_term, "bad equivalent fixture: {name}");
        assert_ne!(old_term, quipu::rdf::value_to_term(&store, &sibling)?);
        if seed_directory.is_none() {
            store.transact(
                &[
                    datum(e, a, legacy.clone(), Op::Assert),
                    datum(e, a, sibling.clone(), Op::Assert),
                ],
                TIME,
                None,
                None,
            )?;
        }
        let export_before =
            String::from_utf8(quipu::rdf::export_rdf(&store, RdfFormat::NTriples)?)?;
        std::fs::write(path.with_extension("nt"), &export_before)?;
        drop(store);
        let mut store = Store::open(path.to_str().ok_or("non-UTF8 path")?)?;
        let query = format!("SELECT ?s WHERE {{ ?s <http://example.org/p> {paired_term} }}");
        let query_rows = quipu::sparql::query(&store, &query)?.rows().len();
        let model_matches = store
            .build_read_model(0)?
            .contains(&store, e, a, &equivalent)?;
        let g = store.overlay_create("http://example.org/probe-overlay-new", 0)?;
        store.overlay_write(g, Op::Tombstone, e, a, equivalent.clone(), TIME)?;
        let hidden_count = store.compose_view(g)?.len();
        store.overlay_write(g, Op::Assert, e, a, equivalent.clone(), TIME)?;
        let revealed_count = store.compose_view(g)?.len();
        store.transact(
            &[datum(e, a, equivalent.clone(), Op::Assert)],
            TIME,
            None,
            None,
        )?;
        let after_repeat = store.current_facts()?.len();
        store.transact(
            &[datum(e, a, equivalent, Op::Retract)],
            "2026-01-02T00:00:00Z",
            None,
            None,
        )?;
        drop(store);
        let store = Store::open(path.to_str().ok_or("non-UTF8 path")?)?;
        let remaining = store.current_facts()?;
        let sibling_survived = remaining
            .iter()
            .any(|f| f.value.to_bytes() == sibling.to_bytes());
        let legacy_still_active = remaining
            .iter()
            .any(|f| f.value.to_bytes() == legacy.to_bytes());
        let mut imported = Store::open_in_memory()?;
        let import_result = quipu::ingest_rdf(
            &mut imported,
            export_before.as_bytes(),
            RdfFormat::NTriples,
            None,
            TIME,
            None,
            None,
        );
        let (import_ok, import_error, roundtrip_terms) = match import_result {
            Ok(_) => (
                true,
                None,
                Some(String::from_utf8(quipu::rdf::export_rdf(
                    &imported,
                    RdfFormat::NTriples,
                )?)?),
            ),
            Err(error) => (false, Some(error.to_string()), None),
        };
        let roundtrip_equal = roundtrip_terms.as_ref().is_some_and(|after| {
            export_before
                .lines()
                .collect::<std::collections::BTreeSet<_>>()
                == after.lines().collect::<std::collections::BTreeSet<_>>()
        });
        // Each false below is an acceptance failure, not a successful fix.
        report.push(serde_json::json!({
            "case": name, "old_exported_term": old_term.to_string(),
            "legacy_bytes": legacy.to_bytes(), "equivalent_exported_term": paired_term.to_string(),
            "query_matches": query_rows, "resident_matches": model_matches,
            "overlay_hidden_count": hidden_count, "overlay_revealed_count": revealed_count,
            "after_repeat_count": after_repeat, "after_retract_count": remaining.len(),
            "sibling_survived": sibling_survived, "legacy_still_active": legacy_still_active,
            "roundtrip_equal": roundtrip_equal, "import_ok": import_ok, "import_error": import_error, "imported_export": roundtrip_terms,
            "acceptance": query_rows == 1 && model_matches && hidden_count == 1
                && revealed_count == 2 && after_repeat == 2 && remaining.len() == 1
                && sibling_survived && !legacy_still_active && import_ok && roundtrip_equal,
        }));
    }
    let output = serde_json::to_string_pretty(&report)?;
    std::fs::write(
        std::path::Path::new(&directory).join("report.json"),
        &output,
    )?;
    println!("{output}");
    Ok(())
}
