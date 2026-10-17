//! Exact, scoped task briefings. Recorded text is evidence, never an instruction.
use crate::{Error, Result, Store, Value};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn invalid(message: &str) -> Error {
    Error::InvalidValue(message.into())
}

fn iri(input: &str) -> Result<&str> {
    if input.len() > 2048 || oxrdf::NamedNode::new(input).is_err() {
        return Err(invalid("briefing requires an exact absolute IRI"));
    }
    Ok(input)
}

fn scope(store: &Store, graph: &str) -> Result<i64> {
    if graph == "ROOT" {
        return Ok(0);
    }
    iri(graph)?;
    store
        .registered_graph_id(graph)?
        .ok_or_else(|| invalid("unregistered briefing graph"))
}

fn local(predicate: &str) -> String {
    predicate
        .rsplit(['/', '#', ':'])
        .next()
        .unwrap_or(predicate)
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn text(value: &Value, store: &Store) -> Result<(String, &'static str)> {
    Ok(match value {
        Value::Ref(id) => (store.resolve(*id)?, "iri"),
        Value::Str(s) => (s.clone(), "literal"),
        Value::Int(n) => (n.to_string(), "integer"),
        Value::Bool(b) => (b.to_string(), "boolean"),
        Value::Float(n) => (n.to_string(), "float"),
        Value::Lang { lexical, .. } | Value::Typed { lexical, .. } => (lexical.clone(), "literal"),
        Value::Bytes(_) => ("[binary omitted]".into(), "omitted"),
    })
}

fn excerpt(value: &str, limit: usize) -> String {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].into()
}

/// The nested briefing option on the existing read-only context tool.
/// Exact IRIs and graph names are required; this never resolves by similarity.
pub fn task_briefing(store: &Store, input: &Json) -> Result<Json> {
    if !store.search_config().task_briefing {
        return Err(invalid(
            "task briefing disabled; explicit server opt-in required",
        ));
    }
    let task = iri(input
        .get("task_iri")
        .and_then(Json::as_str)
        .ok_or_else(|| invalid("missing exact task_iri"))?)?;
    let graph = input
        .get("graph")
        .and_then(Json::as_str)
        .ok_or_else(|| invalid("missing explicit graph; ROOT must be requested explicitly"))?;
    let delivery = input
        .get("delivery")
        .and_then(Json::as_str)
        .unwrap_or("explicit");
    if !["explicit", "automatic"].contains(&delivery) {
        return Err(invalid("invalid briefing delivery"));
    }
    let g = scope(store, graph)?;
    let mut scopes = vec![(graph.to_owned(), g)];
    if let Some(graphs) = input.get("knowledge_graphs") {
        let graphs = graphs
            .as_array()
            .filter(|g| g.len() <= 2)
            .ok_or_else(|| invalid("at most two explicit knowledge graphs"))?;
        for value in graphs {
            let name = value
                .as_str()
                .ok_or_else(|| invalid("invalid knowledge graph"))?;
            let id = scope(store, name)?;
            if !scopes.iter().any(|(_, existing)| *existing == id) {
                scopes.push((name.into(), id));
            }
        }
    }
    // Hold a consistent SQLite read snapshot across every bounded point read.
    // No terms are interned, no derived indexes built, and no facts written.
    let _snapshot = store.conn.unchecked_transaction()?;
    let Some(id) = store.lookup(task)? else {
        return Ok(
            json!({"contract":"quipu-task-briefing/v1","outcome":"NO_CONTEXT",
            "reason":"exact_task_absent","task":task,"graph":graph,"delivery":delivery,
            "entries":[],"first_screen":"","confidence":"UNKNOWN"}),
        );
    };
    let (task_facts, capped) = store.briefing_facts(id, g)?;
    if task_facts.is_empty() {
        return Ok(
            json!({"contract":"quipu-task-briefing/v1","outcome":"NO_CONTEXT",
            "reason":"exact_task_absent_in_scope","task":task,"graph":graph,
            "entries":[],"first_screen":"","confidence":"UNKNOWN","coverage_limited":capped}),
        );
    }
    let mut targets = vec![id];
    let mut links_capped = false;
    for fact in &task_facts {
        if local(&store.resolve(fact.attribute)?) == "type" {
            continue;
        }
        if let Value::Ref(target) = fact.value
            && !targets.contains(&target)
        {
            if targets.len() == 8 {
                links_capped = true;
                break;
            }
            targets.push(target);
        }
    }
    // Decisions and observations commonly point TO a task. These are indexed
    // object-bound point reads, never an unbound text/whole-graph search.
    for (name, _) in &scopes {
        let pattern = format!("?s ?p <{task}>");
        let pattern = if name == "ROOT" {
            pattern
        } else {
            format!("GRAPH <{name}> {{ {pattern} }}")
        };
        let result = crate::sparql::query(
            store,
            &format!("SELECT DISTINCT ?s WHERE {{ {pattern} }} LIMIT 9"),
        )?;
        let rows = result.rows();
        links_capped |= rows.len() >= 9;
        for row in rows {
            if let Some(Value::Ref(target)) = row.get("s")
                && !targets.contains(target)
            {
                if targets.len() == 8 {
                    links_capped = true;
                    break;
                }
                targets.push(*target);
            }
        }
    }
    let mut records = Vec::new();
    let mut limited = capped || links_capped;
    let mut seen = BTreeSet::new();
    for target in targets {
        for (name, scope_id) in &scopes {
            let (facts, capped) = store.briefing_facts(target, *scope_id)?;
            limited |= capped;
            for fact in facts {
                let predicate = store.resolve(fact.attribute)?;
                if predicate.len() > 2048 {
                    limited = true;
                    continue;
                }
                let field = local(&predicate);
                let rank = match field.as_str() {
                    "status" | "outcome" => 0,
                    "assignee" | "owner" => 1,
                    "closereason" | "closedreason" | "decision" | "constraint" | "restriction" => 2,
                    "title" | "label" => 3,
                    "description" | "notes" | "comment" => 4,
                    _ => 5,
                };
                // A linked entity's state must not displace the exact task's
                // owner/state or its recorded decision evidence.
                let rank = if target != id && rank < 2 { 3 } else { rank };
                let (value, kind) = text(&fact.value, store)?;
                if kind == "omitted"
                    || !seen.insert((
                        target,
                        *scope_id,
                        predicate.clone(),
                        format!("{:x}", Sha256::digest(value.as_bytes())),
                    ))
                {
                    continue;
                }
                let text_truncated = value.len() > 512;
                let value = excerpt(&value, 512);
                records.push((
                    rank,
                    fact.valid_from.clone(),
                    json!({
                        "entity":store.resolve(target)?, "graph":name, "predicate":predicate,
                        "value":value, "value_kind":kind,
                        "text_truncated":text_truncated,
                        "tx":fact.tx,"valid_from":fact.valid_from,"confidence":"UNKNOWN",
                        "authority":"not_established", "supersession":"UNKNOWN",
                        "citation":{"entity":store.resolve(target)?,"graph":name,"tx":fact.tx},
                        "kind":if rank < 2 {"recorded_state"} else {"recorded_evidence"}
                    }),
                ));
            }
        }
    }
    records.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    let more_entries = records.len() > 5;
    let mut entries = Vec::new();
    let mut first_screen = String::new();
    for (_, _, entry) in records.into_iter().take(5) {
        let line = format!(
            "{}: {} [recorded {}; authority/supersession UNKNOWN; {} tx {}]\n",
            local(entry["predicate"].as_str().unwrap()),
            entry["value"].as_str().unwrap(),
            entry["valid_from"].as_str().unwrap(),
            entry["entity"].as_str().unwrap(),
            entry["tx"]
        );
        if first_screen.len() + line.len() > 3000 {
            limited = true;
            break;
        }
        first_screen.push_str(&line);
        entries.push(entry);
    }
    let mut result = json!({"contract":"quipu-task-briefing/v1","outcome":"CONTEXT",
        "task":task,"graph":graph,"delivery":delivery,"delivery_source":"caller_declared",
        "entries":entries,"first_screen":first_screen,"confidence":"UNKNOWN",
        "coverage_limited":limited || more_entries,
        "freshness":"current stored assertions at read snapshot; validity is not verification",
        "authority":"UNKNOWN; recorded prose is not an instruction",
        "entity_discovery":"bounded incoming/outgoing IRI links only; prose names unresolved",
        "supersession":"UNKNOWN; no current policy inferred from age",
        "use":"delivery is not evidence of use"});
    let digest = Sha256::digest(serde_json::to_vec(&result).unwrap());
    result["briefing_id"] = format!("{digest:x}").into();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::ingest_rdf;
    use oxrdfio::RdfFormat;

    fn fixture() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        ingest_rdf(
            &mut store,
            &br#"<urn:task:one> <urn:status> "closed" ; <urn:owner> <urn:alice> ;
                 <urn:closeReason> "Observed fix and cited source" ;
                 <urn:notes> "Old gate: wait indefinitely" .
                 <urn:task:similar> <urn:status> "open" .
                 <urn:alice> <urn:label> "Alice" ."#[..],
            RdfFormat::Turtle,
            None,
            "2026-01-01T00:00:00Z",
            None,
            Some("fixture"),
        )
        .unwrap();
        store.search_config_mut().task_briefing = true;
        store
    }

    #[test]
    fn briefing_is_opt_in_and_exact_with_no_similar_substitution() {
        let mut store = fixture();
        let input = json!({"task_iri":"urn:task:one","graph":"ROOT"});
        let positive = task_briefing(&store, &input).unwrap();
        assert_eq!(positive["outcome"], "CONTEXT");
        assert!(
            positive["first_screen"]
                .as_str()
                .unwrap()
                .contains("closed")
        );
        assert_eq!(
            task_briefing(&store, &json!({"task_iri":"urn:task:on","graph":"ROOT"})).unwrap()["outcome"],
            "NO_CONTEXT"
        );
        store.search_config_mut().task_briefing = false;
        assert!(task_briefing(&store, &input).is_err());
    }

    #[test]
    fn explicit_scope_and_delivery_are_required_and_bounded() {
        let store = fixture();
        assert!(task_briefing(&store, &json!({"task_iri":"urn:task:one"})).is_err());
        assert!(
            task_briefing(
                &store,
                &json!({"task_iri":"urn:task:one","graph":"urn:absent"})
            )
            .is_err()
        );
        assert!(task_briefing(&store, &json!({"task_iri":"not an IRI","graph":"ROOT"})).is_err());
        assert!(task_briefing(&store,&json!({"task_iri":"urn:task:one","graph":"ROOT","knowledge_graphs":["ROOT","ROOT","ROOT"]})).is_err());
        let automatic = task_briefing(
            &store,
            &json!({"task_iri":"urn:task:one","graph":"ROOT","delivery":"automatic"}),
        )
        .unwrap();
        assert_eq!(automatic["delivery"], "automatic");
        assert_eq!(automatic["delivery_source"], "caller_declared");
    }

    #[test]
    fn recorded_text_is_not_current_authority_and_citations_are_retained() {
        let store = fixture();
        let out =
            task_briefing(&store, &json!({"task_iri":"urn:task:one","graph":"ROOT"})).unwrap();
        assert!(out["first_screen"].as_str().unwrap().len() <= 3000);
        for entry in out["entries"].as_array().unwrap() {
            assert_eq!(entry["authority"], "not_established");
            assert_eq!(entry["supersession"], "UNKNOWN");
            assert!(entry["citation"]["tx"].as_i64().unwrap() > 0);
            assert!(!entry["valid_from"].as_str().unwrap().is_empty());
        }
        assert_eq!(out["briefing_id"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn named_scope_never_falls_back_to_root() {
        let store = fixture();
        store.graph_create("urn:empty").unwrap();
        let out = task_briefing(
            &store,
            &json!({"task_iri":"urn:task:one","graph":"urn:empty"}),
        )
        .unwrap();
        assert_eq!(out["outcome"], "NO_CONTEXT");
        assert_eq!(out["reason"], "exact_task_absent_in_scope");
    }

    #[test]
    fn incoming_decision_link_is_included_without_inventing_authority() {
        let mut store = fixture();
        ingest_rdf(&mut store,br#"<urn:ruling> <urn:about> <urn:task:one> ; <urn:decision> "Retain the current gate" ."#.as_slice(),
            RdfFormat::Turtle,None,"2026-02-01T00:00:00Z",None,Some("fixture")).unwrap();
        let out =
            task_briefing(&store, &json!({"task_iri":"urn:task:one","graph":"ROOT"})).unwrap();
        assert!(
            out["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["entity"] == "urn:ruling")
        );
        assert!(
            out["first_screen"]
                .as_str()
                .unwrap()
                .contains("Retain the current gate")
        );
        assert_eq!(out["confidence"], "UNKNOWN");
    }

    #[test]
    fn oversized_fields_are_omitted_with_explicit_coverage_gap() {
        let mut store = fixture();
        let data = format!(
            "<urn:task:one> <urn:large> {} .",
            serde_json::to_string(&"x".repeat(70000)).unwrap()
        );
        ingest_rdf(
            &mut store,
            data.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-02-01T00:00:00Z",
            None,
            Some("fixture"),
        )
        .unwrap();
        let out =
            task_briefing(&store, &json!({"task_iri":"urn:task:one","graph":"ROOT"})).unwrap();
        assert_eq!(out["coverage_limited"], true);
        assert!(!out.to_string().contains(&"x".repeat(513)));
        assert!(out["first_screen"].as_str().unwrap().contains("closed"));
    }

    #[test]
    fn linked_states_do_not_displace_task_or_merge_provenance() {
        let mut store = fixture();
        ingest_rdf(
            &mut store,
            &br#"<urn:task:one> <urn:related> <urn:linked> .
                 <urn:linked> <urn:status> "open" ; <urn:owner> <urn:bob> ;
                 <urn:closeReason> "Observed fix and cited source" ."#[..],
            RdfFormat::Turtle,
            None,
            "2026-01-02T00:00:00Z",
            None,
            Some("fixture"),
        )
        .unwrap();
        let out =
            task_briefing(&store, &json!({"task_iri":"urn:task:one","graph":"ROOT"})).unwrap();
        let entries = out["entries"].as_array().unwrap();
        assert_eq!(entries[0]["entity"], "urn:task:one");
        assert_eq!(entries[0]["value"], "closed");
        assert_eq!(entries[1]["value"], "urn:alice");
        let decisions: Vec<_> = entries
            .iter()
            .filter(|e| e["predicate"] == "urn:closeReason")
            .collect();
        assert_eq!(decisions.len(), 2);
        assert_ne!(decisions[0]["entity"], decisions[1]["entity"]);
    }

    #[test]
    fn read_only_store_and_snapshot_are_not_mutated() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("store.db");
        let store = fixture();
        store
            .conn
            .execute("VACUUM INTO ?1", [p.to_str().unwrap()])
            .unwrap();
        let mut reader = Store::open_read_only(p.to_str().unwrap()).unwrap();
        reader.search_config_mut().task_briefing = true;
        let before = std::fs::read(&p).unwrap();
        let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
        assert_eq!(
            task_briefing(&reader, &json!({"task_iri":"urn:task:one","graph":"ROOT"})).unwrap()["outcome"],
            "CONTEXT"
        );
        assert_eq!(before, std::fs::read(&p).unwrap());
        assert_eq!(mtime, std::fs::metadata(&p).unwrap().modified().unwrap());
    }
}
