# Embeddings and Semantic Search

Quipu answers two different kinds of question. **Lexical** retrieval (SPARQL,
`CONTAINS`, exact match) needs nothing but the fact log. **Semantic** retrieval
(`/context`, `quipu_hybrid_search`, `/search`) needs vectors, and vectors need
an embedding provider you configure yourself.

Nothing about semantic retrieval is on by default. This page is the checklist
for turning it on, and — just as important — for telling whether it is on.

## What `knot` does and does not do

This asymmetry surprises people, so it is worth stating plainly:

| Write path | Auto-embeds? |
|------------|--------------|
| `POST /episode` / `quipu_episode` | Yes, when `auto_embed = true` |
| `quipu knot` / `POST /knot` (Turtle ingest) | **No** |
| `quipu-server --embed-backfill` | Yes, all entities, once at startup |
| `POST /embed_backfill` | Yes, all entities, on demand |

A graph loaded from Turtle therefore holds **no** embeddings. Semantic
retrieval over it returns nothing at all — successfully, with a `200` and an
empty result — until you backfill.

```console
$ quipu knot alphax.ttl --db na.db
knotted 2579 facts from alphax.ttl (tx 1)

$ quipu-server --db na.db --embed-backfill   # <- the missing step
```

## Configuring a provider

Two things are required, and having one without the other is the common
failure:

1. **The runtime.** Build with `--features onnx`. This supplies the ONNX
   runtime *only*. It does not supply a model, and a build with the feature on
   is not a build that can embed.
2. **A model on disk, plus the paths to it** in `.bobbin/config.toml`:

```toml
[quipu.embedding]
auto_embed = true
model_path = "models/all-MiniLM-L6-v2/onnx/model.onnx"
tokenizer_path = "models/all-MiniLM-L6-v2/tokenizer.json"
dimension = 384
```

The model files are fetched separately (for example from the
`sentence-transformers/all-MiniLM-L6-v2` repository on Hugging Face). Sandboxed
environments may not have network access to a model host, in which case the
files have to be provisioned into the image or volume ahead of time.

`dimension` must match the model. Both `model_path` and `tokenizer_path` must
be set — with either missing, the server skips provider construction entirely
and starts without embeddings.

## Telling whether it worked

Three signals, in the order you will meet them.

**At startup**, a loaded provider announces itself:

```text
ONNX embedding provider loaded (dim=384, auto_embed=true, deferred)
```

**`--embed-backfill` is fatal when it cannot run.** The flag is an explicit
request for a capability, so a server that cannot honour it exits non-zero
rather than starting up without it. The error names the configuration it
needs. Drop the flag if you deliberately want to serve without embeddings.

**Every retrieval response carries its own status.** `/context`,
`quipu_unified_search`, and `quipu_hybrid_search` all report:

```json
{
  "entities": [],
  "summary": {
    "total_entities": 0,
    "direct_hits": 0,
    "embeddings": { "configured": false, "embedded_entities": 0 }
  }
}
```

Read it as:

| `configured` | `embedded_entities` | Meaning |
|--------------|---------------------|---------|
| `false` | `0` | No provider. Configure `[quipu.embedding]`. |
| `true` | `0` | Provider attached, store never embedded. Run a backfill. |
| `true` | `> 0` | Semantic retrieval is live — an empty result really is "no match". |

That last row is the point of the field: without it, an empty `entities` list
means either "nothing matched" or "this was never going to work", and the two
are indistinguishable from the response alone.

## What degrades without a provider

Only the semantic half. Everything lexical keeps working:

- **Works:** SPARQL (including exact-match grounding), `/query`, `/knot`,
  `/context` text search, link expansion, SHACL, the reasoner.
- **Empty or refused:** vector similarity in `/search` and
  `quipu_hybrid_search`, the `Semantic` relevance hits inside `/context`.

`quipu_hybrid_search` called with a `query` string and no provider returns an
error naming the missing configuration, rather than an empty result set. You
can also bypass the provider entirely by passing a pre-computed `embedding`
array.
