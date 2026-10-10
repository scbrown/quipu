// Browser-only adapters. No runtime, model, or remote request occurs at import.
export class BrowserSemanticSearch {
  #pipeline;
  #loading;
  #indexed;
  #vectors;
  #state = "not_loaded";

  constructor(explorer, { runtimeUrl, modelPath, wasmPath, model = "all-MiniLM-L6-v2" }) {
    this.explorer = explorer;
    this.options = { runtimeUrl, modelPath, wasmPath, model };
    for (const key of ["runtimeUrl", "modelPath", "wasmPath"]) {
      if (!this.options[key]) throw new Error(`${key} must name a self-hosted asset`);
    }
  }

  capabilities() {
    return { backend: "javascript-onnx", state: this.#state, remote_models: false, scope: "ROOT" };
  }

  async #load() {
    if (this.#pipeline) return this.#pipeline;
    if (!this.#loading) {
      this.#state = "loading";
      this.#loading = (async () => {
        const runtime = await import(this.options.runtimeUrl);
        const { env, AutoModel, FeatureExtractionPipeline } = runtime;
        env.allowRemoteModels = false;
        env.allowLocalModels = true;
        env.localModelPath = this.options.modelPath;
        env.backends.onnx.wasm.wasmPaths = this.options.wasmPath;
        env.backends.onnx.wasm.numThreads = 1;
        const options = { device: "wasm", dtype: "q8", local_files_only: true };
        // Load required components explicitly. Automatic file discovery treats
        // self-hosted HTTP assets as absent when Hub access is disabled.
        const readJson = async (file) => {
          const response = await fetch(`${this.options.modelPath.replace(/\/$/, "")}/${this.options.model}/${file}`, {
            credentials: "omit",
          });
          if (!response.ok) throw new Error(`model asset ${file}: HTTP ${response.status}`);
          return response.json();
        };
        const [tokenizerJson, tokenizerConfig, model] = await Promise.all([
          readJson("tokenizer.json"), readJson("tokenizer_config.json"),
          AutoModel.from_pretrained(this.options.model, options),
        ]);
        const Tokenizer = runtime[tokenizerConfig.tokenizer_class?.replace(/Fast$/, "")];
        if (!Tokenizer) throw new Error("model tokenizer class is unsupported");
        const tokenizer = new Tokenizer(tokenizerJson, tokenizerConfig);
        this.#pipeline = new FeatureExtractionPipeline({ task: "feature-extraction", tokenizer, model });
        this.#state = "ready";
        return this.#pipeline;
      })().catch((error) => {
        this.#state = "failed";
        this.#loading = undefined;
        throw error;
      });
    }
    return this.#loading;
  }

  // Query ROOT through the same engine used by the explorer; named receiver
  // policy and staging graphs are never copied into the semantic index.
  #documents() {
    const result = JSON.parse(this.explorer.query(
      "SELECT ?s ?label ?comment WHERE { ?s ?p ?o . " +
      "OPTIONAL { ?s <http://www.w3.org/2000/01/rdf-schema#label> ?label } " +
      "OPTIONAL { ?s <http://www.w3.org/2000/01/rdf-schema#comment> ?comment } }",
    ));
    const documents = new Map();
    const text = (value) => typeof value === "string" ? value : value?.value ?? "";
    for (const row of result.rows) {
      const entity = text(row.s);
      if (!documents.has(entity)) documents.set(entity, new Set());
      for (const value of [row.label, row.comment]) {
        if (text(value)) documents.get(entity).add(text(value));
      }
    }
    return [...documents].sort(([a], [b]) => a.localeCompare(b)).map(([entity, values]) => ({
      entity, text: [...values].sort().join("\n") || entity,
    }));
  }

  async search(query, limit = 10) {
    if (typeof query !== "string" || !query.trim()) throw new Error("query must be nonempty");
    if (!Number.isInteger(limit) || limit < 1 || limit > 100) throw new Error("limit must be 1..100");
    const pipeline = await this.#load();
    const documents = this.#documents();
    const snapshot = JSON.stringify(documents);
    if (snapshot !== this.#indexed) {
      const vectors = [];
      for (const document of documents) {
        const output = await pipeline(document.text, { pooling: "mean", normalize: true });
        vectors.push(Array.from(output.data));
      }
      this.#vectors = vectors;
      this.#indexed = snapshot;
    }
    const vectors = this.#vectors;
    const output = await pipeline(query, { pooling: "mean", normalize: true });
    // A write while inference was in flight makes these results stale. Fail
    // explicitly; a subsequent call rebuilds from the new engine snapshot.
    if (JSON.stringify(this.#documents()) !== snapshot) throw new Error("graph changed during semantic search; retry");
    const vector = Array.from(output.data);
    return documents.map((document, index) => {
      const candidate = vectors[index];
      if (candidate.length !== vector.length) throw new Error("embedding dimensions differ");
      const score = candidate.reduce((sum, value, i) => sum + value * vector[i], 0);
      if (!Number.isFinite(score)) throw new Error("embedding produced a non-finite score");
      return { ...document, score };
    }).sort((a, b) => b.score - a.score || a.entity.localeCompare(b.entity)).slice(0, limit);
  }
}

// Async fetch adapter for an explicitly selected remote. It never imports
// remote facts into the local store or claims a remote supplied its own trust.
export class BrowserRemoteProvider {
  #authToken;

  constructor(engine, { name, endpoint, label = {}, timeoutMs = 30_000, authToken }) {
    if (!name) throw new Error("remote name is required");
    const url = new URL(endpoint, globalThis.location?.href);
    if (!["http:", "https:"].includes(url.protocol)) throw new Error("remote must use HTTP(S)");
    if (url.username || url.password) throw new Error("credentials must not be embedded in a remote URL");
    if (!Number.isInteger(timeoutMs) || timeoutMs < 1) throw new Error("timeout must be positive");
    this.name = name;
    this.endpoint = url.href.replace(/\/$/, "");
    this.config = { ...label, name, url: this.endpoint };
    this.declaredLabel = JSON.parse(engine.remoteLabel(JSON.stringify(this.config)));
    this.timeoutMs = timeoutMs;
    this.#authToken = authToken;
  }

  async query(sparql, graph) {
    const response = await fetch(`${this.endpoint}/query`, {
      method: "POST", headers: {
        "Content-Type": "application/json", "X-Quipu-Client": "browser-explorer",
        ...(this.#authToken ? { Authorization: `Bearer ${this.#authToken}` } : {}),
      },
      body: JSON.stringify({ query: sparql, ...(graph === undefined ? {} : { graph }) }),
      signal: AbortSignal.timeout(this.timeoutMs), credentials: "omit",
    });
    if (!response.ok) throw new Error(`remote ${this.name} refused query: HTTP ${response.status}`);
    return { member: this.name, declared_label: this.declaredLabel, result: await response.json() };
  }
}

// Match native provider-level incompleteness: one failed member does not
// silently erase its outcome or claim the remaining rows are exhaustive.
export class BrowserFederatedProvider {
  constructor(explorer, remotes = [], floor = {}) {
    this.explorer = explorer;
    this.remotes = remotes;
    this.floor = floor;
    const names = ["local", ...remotes.map((remote) => remote.name)];
    if (new Set(names).size !== names.length) throw new Error("federation member names must be distinct");
  }

  async query(sparql) {
    this.explorer.checkFederation(sparql, JSON.stringify(this.remotes.map((r) => r.config)), JSON.stringify(this.floor));
    const members = [{ name: "local", label: null, query: () => JSON.parse(this.explorer.query(sparql)) },
      ...this.remotes.map((remote) => ({ name: remote.name, label: remote.declaredLabel,
        query: async () => (await remote.query(sparql)).result }))];
    const answers = await Promise.allSettled(members.map((member) => Promise.resolve().then(member.query)));
    const rows = [], providers = [];
    let variables;
    const meta = ["_provider", "_trust", "_freshness"];
    const fields = ["_provider"];
    if (members.some((m) => m.label?.trust)) fields.push("_trust");
    if (members.some((m) => m.label?.freshness)) fields.push("_freshness");
    for (const [index, answer] of answers.entries()) {
      const member = members[index];
      let error;
      if (answer.status === "rejected") error = String(answer.reason?.message ?? answer.reason);
      else if (!Array.isArray(answer.value.rows) || !Array.isArray(answer.value.variables)) error = "member did not return SELECT rows";
      else if (!answer.value.variables.every((v) => typeof v === "string") ||
        !answer.value.rows.every((row) => row && typeof row === "object" && !Array.isArray(row))) error = "member SELECT response is malformed";
      else {
        const theirs = answer.value.variables.filter((v) => !meta.includes(v));
        if (variables && JSON.stringify(variables.filter((v) => !meta.includes(v))) !== JSON.stringify(theirs)) error = "member variable list differs";
        else {
          variables ??= [...answer.value.variables, ...fields.filter((v) => !answer.value.variables.includes(v))];
          for (const row of answer.value.rows) rows.push({ ...row,
            ...(!Object.hasOwn(row, "_provider") ? { _provider: member.name } : {}),
            ...(member.label?.trust && !Object.hasOwn(row, "_trust") ? { _trust: member.label.trust.iri } : {}),
            ...(member.label?.freshness && !Object.hasOwn(row, "_freshness") ? { _freshness: member.label.freshness } : {}),
          });
        }
      }
      providers.push({ name: member.name, label: member.label, ok: !error,
        rows: error ? 0 : answer.value.rows.length, error: error ?? null });
    }
    return { result: { variables: variables ?? fields, rows }, providers,
      complete: providers.every((provider) => provider.ok) };
  }
}
