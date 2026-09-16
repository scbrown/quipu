//! Measurement-only worker: one persistent engine and PID for its entire arm.
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::sync::{Arc, Barrier, mpsc};
use std::time::Instant;
use oxigraph::sparql::{CancellationToken, QueryResults, SparqlEvaluator};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
enum Engine { Quipu(Box<quipu::Store>), Oxigraph(oxigraph::store::Store) }
fn emit(v: Value) {
    println!("{v}");
    std::io::stdout().flush().unwrap();
}
fn term(store: &quipu::Store, value: &quipu::types::Value) -> Result<String> {
    use quipu::types::Value as V;
    use oxrdf::Literal;
    Ok(match value {
        V::Ref(id) => {
            let iri = store.resolve(*id)?;
            if iri.starts_with("_:") { iri } else { format!("<{iri}>") }
        }
        V::Str(s) => Literal::new_simple_literal(s.as_str()).to_string(),
        V::Int(n) => Literal::from(*n).to_string(),
        V::Float(n) => Literal::from(*n).to_string(),
        V::Bool(b) => Literal::from(*b).to_string(),
        V::Lang {lexical,lang} => Literal::new_language_tagged_literal(lexical.as_str(),lang.as_str())?.to_string(),
        V::Typed {lexical,datatype} => Literal::new_typed_literal(lexical.as_str(),oxrdf::NamedNode::new(datatype.as_str())?).to_string(),
        V::Bytes(_) => return Err("unexpected bytes binding".into()),
    })
}
struct QueryTimer(mpsc::Sender<()>, Option<std::thread::JoinHandle<()>>);
impl Drop for QueryTimer {
    fn drop(&mut self) {
        let _ = self.0.send(());
        if let Some(thread) = self.1.take() { let _ = thread.join(); }
    }
}
fn query(engine: &Engine, text: &str) -> Result<Value> {
    let start = Instant::now();
    let token = CancellationToken::new();
    let cancel = token.clone();
    let (done, deadline) = mpsc::channel();
    let timer = std::thread::spawn(move || {
        if deadline.recv_timeout(std::time::Duration::from_secs(30)).is_err() { cancel.cancel(); }
    });
    let _timer = QueryTimer(done, Some(timer));
    let mut rows = Vec::new();
    match engine {
        Engine::Quipu(store) => {
            let result = quipu::sparql_query(store, text)?;
            for row in result.rows() {
                let mut out = BTreeMap::new();
                for (k,v) in row { out.insert(k.clone(),term(store,v)?); }
                rows.push(serde_json::to_string(&out)?);
            }
        }
        Engine::Oxigraph(store) => {
            let result = SparqlEvaluator::new().with_cancellation_token(token).parse_query(text)?.on_store(store).execute()?;
            let QueryResults::Solutions(solutions) = result else { return Err("SELECT required".into()); };
            for row in solutions {
                let row = row?;
                let out: BTreeMap<_,_> = row.iter().map(|(k,v)|(k.as_str(),v.to_string())).collect();
                rows.push(serde_json::to_string(&out)?);
            }
        }
    }
    rows.sort_unstable();
    let mut digest = Sha256::new();
    for row in &rows { digest.update((row.len() as u64).to_le_bytes()); digest.update(row.as_bytes()); }
    Ok(json!({"rows":rows.len(),"multiset_sha256":format!("{:x}",digest.finalize()),"wall_seconds":start.elapsed().as_secs_f64()}))
}
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len()!=6 { return Err("engine ARM STORE INPUT GRAPH DECLARED_SHA".into()); }
    let (arm,path,input,graph,sha)=(&args[1],&args[2],&args[3],&args[4],&args[5]);
    let mut engine = match arm.as_str() {
        "quipu" => Engine::Quipu(Box::new(quipu::Store::open(path)?)),
        "oxigraph" => Engine::Oxigraph(oxigraph::store::Store::open(path)?),
        _ => return Err("unknown engine".into()),
    };
    emit(json!({"event":"ready_empty","pid":std::process::id(),"arm":arm}));
    let mut workers = Vec::<mpsc::Sender<(String,Arc<Barrier>,mpsc::Sender<Value>)>>::new();
    for line in std::io::stdin().lock().lines() {
        let cmd: Value=serde_json::from_str(&line?)?;
        match cmd["op"].as_str().unwrap_or("") {
            "load" => {
                let started=Instant::now();
                let reader=std::io::BufReader::new(std::fs::File::open(input)?);
                let count=cmd["declared_count"].as_u64().ok_or("missing count")? as usize;
                match &mut engine {
                    Engine::Quipu(store) => {
                        let g=store.graph_create(graph)?;
                        quipu::ingest_rdf_declared(store,reader,oxrdfio::RdfFormat::NTriples,None,
                            "2026-01-01T00:00:00Z",Some("memory-comparison"),None,g,50_000,
                            &quipu::LoadDeclaration{triples:count,sha256:sha.clone()})?;
                    }
                    Engine::Oxigraph(store) => {
                        let parser=oxrdfio::RdfParser::from_format(oxrdfio::RdfFormat::NTriples)
                            .with_default_graph(oxrdf::NamedNode::new(graph.as_str())?);
                        store.load_from_reader(parser,reader)?;
                        store.flush()?;
                    }
                }
                let ingest_s=started.elapsed().as_secs_f64();
                for _ in 0..4 {
                    let reader=match &engine {
                        Engine::Quipu(store) => {
                            let mut r=quipu::Store::open_read_only(path)?;
                            r.adopt_read_config_from(store);
                            Engine::Quipu(Box::new(r))
                        }
                        Engine::Oxigraph(store) => Engine::Oxigraph(store.clone()),
                    };
                    let (tx,rx)=mpsc::channel::<(String,Arc<Barrier>,mpsc::Sender<Value>)>();
                    workers.push(tx);
                    std::thread::spawn(move || {
                        for (text,barrier,reply) in rx {
                            barrier.wait();
                            let started=Instant::now();
                            let result=match query(&reader,&text) { Ok(v)=>v,Err(e)=>json!({"error":e.to_string(),"wall_seconds":started.elapsed().as_secs_f64()}) };
                            if reply.send(result).is_err() { break; }
                        }
                    });
                }
                let persisted=match &engine { Engine::Oxigraph(s)=>Some(s.len()?),_=>None };
                emit(json!({"event":"ready_loaded","pid":std::process::id(),"ingest_wall_seconds":ingest_s,"oxigraph_quads":persisted,"reader_count":workers.len()}));
            }
            "query" => {
                let n=cmd["concurrency"].as_u64().ok_or("missing concurrency")? as usize;
                if n==0 || n>workers.len() { return Err("invalid concurrency or not loaded".into()); }
                let text=cmd["query"].as_str().ok_or("missing query")?;
                let barrier=Arc::new(Barrier::new(n));
                let (tx,rx)=mpsc::channel();
                for worker in workers.iter().take(n) { worker.send((text.into(),barrier.clone(),tx.clone()))?; }
                drop(tx);
                let results: Vec<_>=rx.into_iter().collect();
                emit(json!({"event":"query_done","pid":std::process::id(),"results":results}));
            }
            "stop" => break,
            _ => return Err("unknown operation".into()),
        }
    }
    Ok(())
}
