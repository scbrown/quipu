# Literal identity and legacy storage compatibility

Status: candidate implementation, pending compatibility and operational review.

RDF literal identity consists of lexical form and datatype (plus a language tag
for language-tagged literals). Numeric values are derived separately. For example,
`"01"^^xsd:integer` and `"1"^^xsd:integer` compare numerically equal while remaining
different terms. Ill-typed literals remain valid RDF terms and retain their text.
See [RDF 1.1 Concepts](https://www.w3.org/TR/rdf11-concepts/#section-Graph-Literal)
and [SPARQL sameTerm](https://www.w3.org/TR/sparql11-query/#func-sameTerm).

## Encoding and lookup

The tagged-blob codec is unchanged. Canonical integer spellings that fit `i64`
remain `Int`; exact `true` and `false` remain `Bool`. Other input uses the existing
`Typed` encoding with the original lexical form and datatype. RDF ingestion and
SPARQL constants share the converter. Lost historical spellings cannot be recovered.

`Value::term_key` identifies the exported RDF term independently of physical
encoding. `PartialEq`, joins, grouping, sameTerm, overlay composition and resident
fingerprints use term identity. Numeric equality and comparisons derive values;
integer and decimal comparisons use arbitrary precision. Integer/decimal arithmetic, SUM, AVG and ordering also avoid binary floating
point. Decimal division uses the decimal library's default precision for
nonterminating results; floating-point operands retain floating promotion.

A legacy `Float` retains its exact encoded bits and its existing Rust-rendered
export text. Thus `Float(1.0)` and `Typed("1", xsd:double)` are one term, while
`Typed("1.0", xsd:double)` is another. Signed zero remains lexically distinct.
The historical `inf` spelling remains `inf`, even though the XSD spelling is
`INF`; compatibility does not rewrite an ill-typed exported term into a valid one.

Bound SQL queries enumerate compatible blobs and use the existing value index.
NaN has many physical bit patterns with the same exported `NaN` text. Its lookup
uses the indexed BLOB range `[03,04)` and decodes candidates, including attached
layers when querying them. This scans **all stored Float encodings**, including
historical rows. Its scale cost must be measured before operational acceptance;
calling it indexed is not a bounded-cost claim.

## Retraction and notifications

Logical retraction closes every active physical encoding of the selected term
in the selected graph. Audit retraction rows retain each original encoding.
Source and episode cleanup explicitly restrict closure to the owning source;
the source restriction is passed separately from the new transaction's provenance.
Replacing one snapshot cannot retract another producer's equivalent encoding.
A named producer asserting an already-visible term retains its own physical claim;
idempotence is per source across compatible encodings. The second claim produces
no logical assertion event, and removing the first produces no logical retraction.
Anonymous assertions retain global idempotence because they name no cleanup owner.
History and source-claim queries expose the separate provenance rows. Current
graph facts, SPARQL and the resident model project RDF term identity; a current
fact carries representative provenance, not the full list of owners.

The event log, reactive observer delta and resident maintenance receive a logical
retraction only when the last active encoding disappears. Repeated operations on
one term in a batch report its committed before/after transition, so an equivalent
retract/assert pair cannot remove a term from the resident model. A pooled reader catching
up projects the final presence of each touched term. Physical closures remain in
the fact history even when logical presence does not change.

Overlay tombstones and shadowing use term keys. Nearest-overlay composition and
governed-parent composition retain their distinct precedence rules. Neither
mutates the parent. Import of physical history keeps its raw encodings; query
projection and subsequent logical writes reconcile equivalent representations.

## Compatibility and rollback

There is no schema, tag or persistent index migration. Old readers can decode all
newly written variants. Returning to an old binary restores its known lexical and
alias defects; it does not require rewriting stored values. Published physical
snapshot bytes and old Float export spellings are not reinterpreted.

Before landing, require an old-build database fixture covering Float positive and
negative zero, finite fractions, infinities, multiple NaN payloads, and direct
Typed canonical integer/boolean encodings. Check bound queries, resident reads,
repeat assertion, exact retract, overlay hide/reveal, export/import and reopen,
with a different-spelling sibling as a control. Source cleanup must retain a
second producer's assertion, including through pooled-reader catchup and all
notification paths. The seven Turtle evaluation failures and the existing suites
must pass. Performance, complete CI, owner review and the deployment slot remain
separate acceptance gates.

The offline `literal_compatibility` example emits a JSON result per legacy case.
Its first argument is a new output directory; an optional second directory
supplies databases written by the old probe. It refuses an existing output
location. A successful process exit means the probe completed; inspect each
`acceptance` field to assess the candidate.
