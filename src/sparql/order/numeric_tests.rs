use super::*;
#[cfg(test)]
mod ian_numeric_falsifier {
    use super::*;
    fn ids(rows: &[Bindings]) -> Vec<i64> {
        rows.iter()
            .map(|r| match r["id"] {
                Value::Int(n) => n,
                _ => panic!("id"),
            })
            .collect()
    }
    const QUERY: &str = "SELECT ?x ?id WHERE { VALUES (?v ?id) { (9007199254740993 0) (-9007199254740992e0 1) (9007199254740992 2) } BIND(IF(?id = 1, -?v, ?v) AS ?x) } ORDER BY ?x";
    #[test]
    fn ian_numeric_homogeneous_positive() {
        let store = Store::open_in_memory().unwrap();
        let q = QUERY.replace("-9007199254740992e0", "-9007199254740992");
        let all = crate::sparql::query(&store, &q).unwrap();
        let limited = crate::sparql::query(&store, &format!("{q} LIMIT 2")).unwrap();
        assert_eq!(ids(all.rows()), vec![1, 2, 0]);
        assert_eq!(ids(limited.rows()), ids(&all.rows()[..2]));
    }
    #[test]
    fn ian_numeric_public_mixed_promotion_prefix() {
        let store = Store::open_in_memory().unwrap();
        let all = crate::sparql::query(&store, QUERY).unwrap();
        let limited = crate::sparql::query(&store, &format!("{QUERY} LIMIT 2")).unwrap();
        println!(
            "public complete={:?}; LIMIT2={:?}; values={:?}",
            ids(all.rows()),
            ids(limited.rows()),
            all.rows()
        );
        assert_eq!(ids(all.rows()), vec![2, 0, 1]);
        assert!(
            matches!(all.rows()[2]["x"], Value::Float(_)),
            "real mixed native numeric types"
        );
        assert_eq!(
            ids(limited.rows()),
            ids(&all.rows()[..2]),
            "mixed numeric LIMIT changed full-sort prefix"
        );
    }
    #[test]
    fn mixed_numeric_offset_and_desc_match_full_sort() {
        let store = Store::open_in_memory().unwrap();
        for query in [
            QUERY.to_owned(),
            QUERY.replace("ORDER BY ?x", "ORDER BY DESC(?x)"),
        ] {
            let all = crate::sparql::query(&store, &query).unwrap();
            for offset in 0..3 {
                let limited =
                    crate::sparql::query(&store, &format!("{query} LIMIT 1 OFFSET {offset}"))
                        .unwrap();
                assert_eq!(limited.rows(), &all.rows()[offset..offset + 1]);
            }
        }
    }
    #[test]
    fn ian_numeric_direct_heap_prefix() {
        let store = Store::open_in_memory().unwrap();
        let rows = vec![
            Bindings::from([
                ("id".into(), Value::Int(0)),
                ("x".into(), Value::Int(9007199254740993)),
            ]),
            Bindings::from([
                ("id".into(), Value::Int(1)),
                ("x".into(), Value::Float(9007199254740992.0)),
            ]),
            Bindings::from([
                ("id".into(), Value::Int(2)),
                ("x".into(), Value::Int(9007199254740992)),
            ]),
        ];
        let expr = [OrderExpression::Asc(
            spargebra::algebra::Expression::Variable(spargebra::term::Variable::new("x").unwrap()),
        )];
        let all = sort_rows(&store, rows.clone(), &expr, &TemporalContext::default()).unwrap();
        let top = top_rows(&store, rows, &expr, &TemporalContext::default(), 2).unwrap();
        println!("direct complete={:?}; top2={:?}", ids(&all), ids(&top));
        assert_eq!(ids(&all), vec![2, 0, 1]);
        assert_eq!(ids(&top), ids(&all[..2]));
    }
}
