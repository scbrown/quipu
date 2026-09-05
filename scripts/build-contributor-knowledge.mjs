// Public, source-backed knowledge for the repository share. No service reads.
// Curated excerpts are intentional: repository docs also contain private examples.
import { readFileSync, existsSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const [source, output] = process.argv.slice(2);
if (!source || !output) throw new Error("usage: build-contributor-knowledge.mjs <repo> <output.ttl>");
const root = resolve(source);
const registry = JSON.parse(readFileSync(resolve(root, "docs/knowledge/contributor-stories.json")));
const ns = "https://quipu.dev/knowledge/";
// Read the established code namespace from the producer's query, so this
// projection and Bobbin's identity remain the same definition.
const query = readFileSync(resolve(root, "queries/repository-share-quipu.rq"), "utf8");
const codeNS = query.match(/PREFIX aegis: <([^>]+)>/)[1];
const iri = (s) => `<${s}>`;
const lit = (s) => JSON.stringify(s);
const lines = [];
const fact = (s, p, o, literal = false) => lines.push(`${iri(s)} ${iri(p)} ${literal ? lit(o) : iri(o)} .`);
const rdf = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const label = "http://www.w3.org/2000/01/rdf-schema#label";
const comment = "http://www.w3.org/2000/01/rdf-schema#comment";
const prov = "http://www.w3.org/ns/prov#wasDerivedFrom";
const node = (id, type, title, summary) => {
  fact(ns + id, rdf, type);
  fact(ns + id, label, title, true);
  if (summary) fact(ns + id, comment, summary, true);
};
const doc = (path) => codeNS + "doc/quipu/" + encodeURIComponent(path);
const publicSource = (path) => "https://github.com/scbrown/quipu/blob/main/" + path;
const evidence = (id, entry) => {
  const text = readFileSync(resolve(root, entry.source), "utf8").replace(/\s+/g, " ");
  if (!text.includes(entry.quote.replace(/\s+/g, " "))) {
    throw new Error(`source excerpt changed: ${entry.source} (${id}); review the story`);
  }
  fact(ns + id, ns + "excerpt", entry.quote, true);
  fact(ns + id, prov, publicSource(entry.source));
  fact(ns + id, ns + "explainedIn", doc(entry.source));
};
node("vision", ns + "Vision", registry.vision.title, registry.vision.summary);
evidence("vision", registry.vision);
for (const s of registry.stories) {
  node(s.id, codeNS + "DecisionRecord", s.title, s.summary);
  evidence(s.id, s);
  fact(ns + "vision", ns + "guides", ns + s.id);
  node(s.id + "-episode", ns + "Episode", "Story: " + s.title, s.episode);
  fact(ns + s.id + "-episode", prov, publicSource(s.source));
  fact(ns + s.id, ns + "learnedFrom", ns + s.id + "-episode");
  for (const path of s.code) {
    if (!existsSync(resolve(root, path))) throw new Error(`missing code witness: ${path}`);
    fact(ns + s.id, ns + "governs", codeNS + "code/quipu/" + encodeURIComponent(path));
  }
}
node("trust-directive", codeNS + "Directive", "Verify before trust", registry.stories[3].quote);
evidence("trust-directive", registry.stories[3]);
fact(ns + "verify-before-trust", ns + "requires", ns + "trust-directive");
node("book", ns + "Book", "Contributor's book", "The book's reading order, linked to its indexed documents.");
fact(ns + "vision", ns + "explainedIn", ns + "book");
let section = "Introduction", previous = null, order = 0;
for (const line of readFileSync(resolve(root, "docs/book/src/SUMMARY.md"), "utf8").split("\n")) {
  if (/^# /.test(line)) section = line.slice(2);
  const match = line.match(/^\s*(?:- )?\[([^\]]+)\]\(([^)]+\.md)\)/);
  if (!match) continue;
  const [, title, path] = match;
  if (!existsSync(resolve(root, "docs/book/src", path))) throw new Error(`missing chapter: ${path}`);
  const id = "chapter/" + encodeURIComponent(path);
  node(id, ns + "Chapter", title);
  fact(ns + id, ns + "topic", section, true);
  fact(ns + id, ns + "order", String(order++), true);
  fact(ns + id, ns + "explainedIn", doc("docs/book/src/" + path));
  fact(ns + "book", ns + "chapter", ns + id);
  if (previous) fact(ns + previous, ns + "next", ns + id);
  previous = id;
}
// Fail on private literals in the curated payload, rather than silently redact
// them into misleading prose. Existing vocabulary IRIs are not deployment URLs.
for (const line of lines.filter((l) => l.includes('> "'))) {
  if (/(?:\b[\w-]+\.(?:svc|lan)\b|\b(?:10|192\.168|172\.(?:1[6-9]|2\d|3[01]))\.\d|\/home\/|\baegis-\w+)/i.test(line)) {
    throw new Error("private identifier in contributor literal");
  }
}
writeFileSync(output, lines.sort().join("\n") + "\n");
console.log(`Contributor knowledge: ${lines.length} facts, ${registry.stories.length} stories, ${order} chapters`);
