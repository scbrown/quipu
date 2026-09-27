# Scrubbed illustrative evidence

These are editorial paraphrases of selected historical item views, not the
requests sent to the model. Names, addresses, identifiers and personal context
are removed. They illustrate the evidence and its limitations; the numeric
response artifacts preserve the actual decisions.

| Item | Evidence synopsis | Gold | Pilot decision at .75 |
|---|---|---|---|
| p-016 | Two records describe the same entertainment dashboard, one as a supervised application and the other as its HTTP route; descriptions agree on application and serving endpoint. | same | same (.88) |
| p-083 | Two failure records describe code entities disconnected from repository ownership because repository identity was stored as a literal rather than an edge. One is a detailed diagnosis, the other a brief repair account. | same | abstain (.59) |
| n-b383d6d2a38889f65059a5a1 | An entertainment dashboard and a static pages server share a web-application type but have distinct functions. | different | different |
| n-d7a61edb4bdd3ce7afc8c51d | An event-processing remediator and an endpoint-probing exporter share a supervised-service type but perform distinct work. | different | different |

The semantic negatives above are easy. Selecting same-type neighbours does not
make every surviving independently verified negative difficult. These examples
must not be used to imply broad hard-negative discrimination.

`item-views.json` releases the full item set as structural views. Private string
values are replaced by randomly assigned opaque tokens; descriptions are withheld
with their character counts. The private token correspondence is not published.
These views preserve field presence and repeated values, not lexical similarity
or semantic content. The exact prompt, frozen original baseline scores, labels,
and raw numeric responses support offline recomputation of reported metrics.
They do not enable independent regeneration of embeddings or model inference
from the original private evidence. This is a reproducibility limitation, not an
anonymized request set claimed to be equivalent to the originals.
