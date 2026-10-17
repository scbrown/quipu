use super::Request;
use super::judgment::{self, Claim, Entry, Judgment, Record};
use super::ledger::{entity, invalid, ns, reference, retained_subjects, scalar, subjects};
use crate::{Result, Store, Value, governance::decision_seal, namespace::RDF_TYPE};

const RDF_VALUE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#value";
const LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

pub(super) fn read(store: &Store, request: &Request) -> Result<Judgment> {
    let mut statement = store.conn.prepare(
        "SELECT turtle FROM shapes WHERE name = 'rule-answers' AND valid_to IS NULL LIMIT 2",
    )?;
    let profiles: Vec<String> = statement
        .query_map([], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    if profiles.as_slice() != [judgment::PROFILE] {
        return Err(invalid(
            "reviewed purpose profile is unavailable or differs",
        ));
    }
    let claim = Claim::from(request);
    let digest = claim.digest()?;
    let mut entries = Vec::new();
    for decision in retained_subjects(store, judgment::MARKER, &Value::Str(digest.clone()))? {
        entries.push(read_entry(store, request, &claim, &digest, &decision)?);
    }
    judgment::fold(entries)
}

fn read_entry(
    store: &Store,
    request: &Request,
    claim: &Claim,
    digest: &str,
    decision: &str,
) -> Result<Entry> {
    let facts = entity(store, decision)?;
    // Seal CBD traversal is recursive for blank nodes. This profile supports
    // bounded scalar carriers only, so reject indirection BEFORE that traversal.
    let scalar_fields = [
        RDF_VALUE.to_string(),
        LABEL.to_string(),
        judgment::MARKER.to_string(),
        ns("decisionPolicy"),
        ns("question"),
        ns("option"),
        ns("outcome"),
        ns("by"),
        ns("evidenceHash"),
        ns("gate"),
    ];
    let decision_type = store.lookup(&ns("Decision"))?;
    let mut typed = false;
    for (predicate, value) in &facts {
        if predicate == RDF_TYPE {
            if !matches!(value, Value::Ref(id) if Some(*id) == decision_type) {
                return Err(invalid("unexpected Decision type"));
            }
            typed = true;
        } else if scalar_fields.contains(predicate) {
            if !matches!(value, Value::Str(_)) {
                return Err(invalid("non-scalar sealed field"));
            }
        } else if predicate != &ns("signature")
            && predicate != "http://www.w3.org/ns/prov#wasGeneratedBy"
        {
            return Err(invalid("unsupported sealed field"));
        }
    }
    if !typed
        || scalar(&facts, judgment::MARKER)? != digest
        || scalar(&facts, &ns("decisionPolicy"))? != request.judgment_policy
    {
        return Err(invalid("Decision claim/type/policy mismatch"));
    }
    if !facts.iter().any(|(predicate, value)| {
        predicate == LABEL && matches!(value, Value::Str(s) if !s.trim().is_empty())
    }) {
        return Err(invalid("missing named Decision label"));
    }
    let record: Record = serde_json::from_str(&scalar(&facts, RDF_VALUE)?)
        .map_err(|_| invalid("malformed strict judgment record"))?;
    if record.protocol != "quipu.rule-judgment/v1"
        || &record.claim != claim
        || record.authority.trim().is_empty()
        || record.reason.trim().is_empty()
    {
        return Err(invalid(
            "judgment record identity/authority/reason mismatch",
        ));
    }
    if scalar(&facts, &ns("outcome"))? != "approve"
        || scalar(&facts, &ns("by"))? != record.authority
        || scalar(&facts, &ns("evidenceHash"))? != format!("sha256:{digest}")
    {
        return Err(invalid(
            "Decision approval carrier does not endorse this claim and authority",
        ));
    }
    let decision_id = store
        .lookup(decision)?
        .ok_or_else(|| invalid("missing Decision"))?;
    let decision_digest = decision_seal::decision_digest(store, decision)?;
    let mut verdicts = Vec::new();
    for presentation in subjects(store, &ns("forDecision"), &Value::Ref(decision_id))? {
        // Bound all entities the seal's scalar readers inspect before calling it.
        let presentation_facts = entity(store, &presentation)?;
        let nonce = scalar(&presentation_facts, &ns("nonce"))?;
        if nonce.len() != 64
            || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
            || presentation != ns(&format!("presentation_{nonce}"))
        {
            return Err(invalid("presentation identity does not match native nonce"));
        }
        if reference(&presentation_facts, &ns("forDecision"))? != decision_id
            || scalar(&presentation_facts, &ns("sealedDigest"))? != decision_digest
        {
            return Err(invalid("presentation does not seal this exact Decision"));
        }
        let presentation_id = store
            .lookup(&presentation)?
            .ok_or_else(|| invalid("missing presentation"))?;
        for verdict in subjects(store, &ns("forPresentation"), &Value::Ref(presentation_id))? {
            let verdict_facts = entity(store, &verdict)?;
            if reference(&verdict_facts, &ns("forPresentation"))? != presentation_id {
                return Err(invalid("verdict does not name this exact presentation"));
            }
            if scalar(&verdict_facts, &ns("verifier"))? != record.authority {
                return Err(invalid("authority differs from admitted verifier"));
            }
            if scalar(&verdict_facts, &ns("outcome"))? != "approve"
                || decision_seal::verify_recorded(store, &verdict)?.is_err()
            {
                return Err(invalid(
                    "judgment seal was not admitted or no longer verifies",
                ));
            }
            verdicts.push(verdict);
            if verdicts.len() > 16 {
                return Err(invalid("verdict cap exceeded"));
            }
        }
    }
    if verdicts.is_empty() {
        return Err(invalid("Decision has no admitted judgment"));
    }
    verdicts.sort();
    verdicts.dedup();
    Ok(Entry {
        decision: decision.into(),
        decision_digest,
        verdicts,
        record,
    })
}
