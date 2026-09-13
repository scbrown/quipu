//! Compare declared top-level fields with syntax-level handler reads.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::visit::{self, Visit};

#[derive(Default)]
struct Reads {
    roots: BTreeSet<String>,
    keys: BTreeSet<String>,
    calls: BTreeSet<String>,
}

impl Reads {
    fn root(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Path(p) => p
                .path
                .get_ident()
                .is_some_and(|i| self.roots.contains(&i.to_string())),
            syn::Expr::Reference(r) => self.root(&r.expr),
            _ => false,
        }
    }
    fn key(&mut self, expr: &syn::Expr) {
        if let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) = expr
        {
            self.keys.insert(s.value());
        }
    }
}
impl<'ast> Visit<'ast> for Reads {
    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        if e.method == "get"
            && self.root(&e.receiver)
            && let Some(key) = e.args.first()
        {
            self.key(key);
        }
        visit::visit_expr_method_call(self, e);
    }
    fn visit_expr_index(&mut self, e: &'ast syn::ExprIndex) {
        if self.root(&e.expr) {
            self.key(&e.index);
        }
        visit::visit_expr_index(self, e);
    }
    fn visit_expr_call(&mut self, e: &'ast syn::ExprCall) {
        if e.args.iter().any(|a| self.root(a)) {
            if let syn::Expr::Path(p) = &*e.func
                && let Some(name) = p.path.segments.last()
            {
                self.calls.insert(name.ident.to_string());
            }
            // required_str(input, "name") and analogous field helpers.
            if e.args.first().is_some_and(|a| self.root(a))
                && let Some(key) = e.args.iter().nth(1)
            {
                self.key(key);
            }
        }
        visit::visit_expr_call(self, e);
    }
}

fn functions(dir: &Path, found: &mut BTreeMap<String, Reads>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            functions(&path, found);
            continue;
        }
        if path.extension().is_none_or(|e| e != "rs")
            || path.file_name().unwrap().to_string_lossy().contains("test")
        {
            continue;
        }
        let ast = syn::parse_file(&std::fs::read_to_string(&path).unwrap()).unwrap();
        for item in ast.items {
            let syn::Item::Fn(f) = item else { continue };
            let mut reads = Reads::default();
            for arg in &f.sig.inputs {
                if let syn::FnArg::Typed(a) = arg {
                    let ty = match &*a.ty {
                        syn::Type::Reference(r) => &*r.elem,
                        ty => ty,
                    };
                    let is_json = matches!(ty, syn::Type::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "JsonValue"));
                    let is_json_wrapper = matches!(ty, syn::Type::Path(p) if p.path.segments.last().is_some_and(|s|
                        s.ident == "Json" && matches!(&s.arguments, syn::PathArguments::AngleBracketed(args) if args.args.iter().any(|arg|
                            matches!(arg, syn::GenericArgument::Type(syn::Type::Path(t)) if t.path.is_ident("JsonValue"))))));
                    if is_json_wrapper && let syn::Pat::TupleStruct(p) = &*a.pat {
                        for pat in &p.elems {
                            if let syn::Pat::Ident(i) = pat {
                                reads.roots.insert(i.ident.to_string());
                            }
                        }
                    }
                    if is_json && let syn::Pat::Ident(i) = &*a.pat {
                        reads.roots.insert(i.ident.to_string());
                    }
                }
            }
            reads.visit_block(&f.block);
            let target = found.entry(f.sig.ident.to_string()).or_default();
            target.keys.extend(reads.keys);
            target.calls.extend(reads.calls);
        }
    }
}

#[test]
fn audit_schema_fields() {
    let mut fs = BTreeMap::new();
    functions(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut fs);
    let mut expanded: BTreeMap<_, _> = fs
        .iter()
        .map(|(n, r)| (n.clone(), r.keys.clone()))
        .collect();
    loop {
        let old = expanded.clone();
        for (name, reads) in &fs {
            for callee in &reads.calls {
                if let Some(keys) = old.get(callee) {
                    expanded.get_mut(name).unwrap().extend(keys.clone());
                }
            }
        }
        if expanded == old {
            break;
        }
    }
    let mut gaps = BTreeMap::new();
    for def in quipu::tool_definitions() {
        let name = def["name"].as_str().unwrap();
        let function = if name == "quipu_explain" {
            "explain".to_string()
        } else if name == "quipu_graph" {
            "tool_graph_view".to_string()
        } else {
            format!("tool_{}", name.strip_prefix("quipu_").unwrap())
        };
        let declared: BTreeSet<String> = def["inputSchema"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        if let Some(read) = expanded.get(&function) {
            let missing: Vec<_> = read.difference(&declared).collect();
            if !missing.is_empty() {
                gaps.insert(name.to_string(), serde_json::json!(missing));
            }
        } else {
            gaps.insert(name.to_string(), serde_json::json!({"unmapped":function}));
        }
    }
    assert!(
        gaps.is_empty(),
        "undeclared handler reads: {}",
        serde_json::to_string_pretty(&gaps).unwrap()
    );
}

#[test]
fn audit_detects_multiline_index_and_helper_reads_but_not_nested_keys() {
    let block: syn::Block = syn::parse_quote!({
        input.get("turtle");
        input["timestamp"];
        required_str(&input, "source");
        input.get("nodes").map(|node| node.get("name"));
    });
    let mut reads = Reads::default();
    reads.roots.insert("input".into());
    reads.visit_block(&block);
    assert_eq!(
        reads.keys,
        ["nodes", "source", "timestamp", "turtle"]
            .map(String::from)
            .into()
    );
    assert!(reads.calls.contains("required_str"));
}

#[test]
fn episode_struct_fields_are_declared() {
    let ast = syn::parse_file(include_str!("../src/episode/mod.rs")).unwrap();
    let episode = ast
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(s) if s.ident == "Episode" => Some(s),
            _ => None,
        })
        .expect("Episode struct must be found");
    let definitions = quipu::tool_definitions();
    let schema = definitions
        .iter()
        .find(|d| d["name"] == "quipu_episode")
        .unwrap();
    let properties = schema["inputSchema"]["properties"].as_object().unwrap();
    for field in &episode.fields {
        let name = field.ident.as_ref().unwrap().to_string();
        assert!(
            properties.contains_key(&name),
            "Episode field {name} is undeclared"
        );
    }
    assert!(
        !properties.contains_key("timestamp"),
        "legacy ignored timestamp must be reported"
    );
}

#[test]
fn http_only_fields_match_handler_reads() {
    let mut fs = BTreeMap::new();
    functions(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut fs);
    let source = syn::parse_file(include_str!("../src/server/input_fields.rs")).unwrap();
    let fields = source
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Const(c) if c.ident == "HTTP_ONLY_FIELDS" => Some(&*c.expr),
            _ => None,
        })
        .expect("HTTP field declarations");
    let syn::Expr::Reference(r) = fields else {
        panic!("expected reference")
    };
    let syn::Expr::Array(array) = &*r.expr else {
        panic!("expected array")
    };
    assert_eq!(
        array.elems.len(),
        4,
        "audit new HTTP-only handlers explicitly"
    );
    for entry in &array.elems {
        let syn::Expr::Tuple(tuple) = entry else {
            panic!("expected tuple")
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(name),
            ..
        }) = &tuple.elems[0]
        else {
            panic!("expected name")
        };
        let function = match name.value().as_str() {
            "quipu_subscriptions" => "tool_subscriptions",
            "quipu_reason" => "reason",
            "quipu_graph_create" => "tool_graph_create",
            "quipu_graph_label" => "tool_graph_label",
            other => panic!("unmapped HTTP handler {other}"),
        };
        let syn::Expr::Reference(r) = &tuple.elems[1] else {
            panic!("expected fields reference")
        };
        let syn::Expr::Array(fields) = &*r.expr else {
            panic!("expected fields array")
        };
        let declared: BTreeSet<_> = fields
            .elems
            .iter()
            .map(|field| {
                let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(key),
                    ..
                }) = field
                else {
                    panic!("expected field name")
                };
                key.value()
            })
            .collect();
        assert_eq!(
            declared,
            fs.get(function).expect("handler found").keys,
            "{function}"
        );
    }
}
