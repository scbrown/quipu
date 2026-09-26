# Blank-node scope on RDF load

Ordinary RDF loads scope blank nodes to the complete input bytes and destination
graph. Two documents using `_:b0` no longer merge that node. Identical input in
the same graph is idempotent, including anonymous `[]` nodes and collections;
this changes the previous behavior of reloading anonymous nodes.

The loader first spools the entire input to a private temporary file while
hashing it. Parsing then uses one map from parser node IDs to first-encounter
indices for the whole document, including across transaction chunks. Memory
grows with distinct blank nodes, rather than document bytes; temporary disk
space grows with document bytes. Browser Wasm builds use an in-memory spool,
so those builds require memory proportional to input size. Spool errors fail before facts are committed.
Existing chunked parse-error and declaration-failure behavior remains unchanged.

`quipu knot` and `quipu ingest` accept `--blank-node-scope ID`. `/knot` and the
MCP tool accept `blank_node_scope`. Library callers can use the `_with_scope`
variants in `quipu::rdf`. A different ID separates deliberate repeat loads of
identical content. The same ID shares blank nodes across destination graphs
**only for identical input bytes**. Editing or reordering the input changes its
hash and therefore changes its blank nodes, even with the same explicit ID.
Empty scope IDs are refused. A scope is not authority to write another graph.

Generated labels remain RDF blank-node labels on export. They are opaque internal
identifiers: consumers must not interpret their hash or index as application
data. A golden fixture pins the parser encounter order for an explicit label,
anonymous and nested nodes, a list, and a node spanning transaction chunks.
Parser changes that alter this order require compatibility review.

Internal share deltas retain their already assigned identities; they are not
new document loads. Existing stored blank-node identifiers are not rewritten.
No migration or repair of previously merged nodes is attempted.
