// Run on the RECEIVER after import and promotion, not on producer input.
import { execFileSync } from "node:child_process";
const [binary, db] = process.argv.slice(2);
const ask = (query) => {
  // The native query CLI prints TSV, not the REST/wasm JSON result. These
  // projections contain only IRIs and counts, never multiline prose.
  const text = execFileSync(binary, ["query", query, "--db", db], { encoding: "utf8" }).trim();
  const lines = text.split("\n"), footer = lines.pop().match(/^(\d+) results$/);
  if (!footer || !/^-+$/.test(lines[1])) throw new Error("unrecognized query output");
  const columns = lines[0].split("\t");
  const rows = lines.slice(2).filter(Boolean).map((line) => {
    const values = line.split("\t");
    if (values.length !== columns.length) throw new Error("invalid query row");
    return Object.fromEntries(columns.map((key, i) => [key, values[i]]));
  });
  if (rows.length !== Number(footer[1])) throw new Error("query row count mismatch");
  return rows;
};
const K = "https://quipu.dev/knowledge/";
const types = ask(`SELECT ?type (COUNT(?s) AS ?n) WHERE { ?s a ?type . FILTER(STRSTARTS(STR(?s), "${K}")) } GROUP BY ?type`);
for (const suffix of ["Vision", "Episode", "DecisionRecord", "Directive", "Book", "Chapter"]) {
  if (!types.some((r) => r.type.endsWith(suffix) && Number(r.n) > 0)) throw new Error(`missing ${suffix}`);
}
const paths = ask(`SELECT ?decision ?module WHERE { <${K}vision> <${K}guides> ?decision . ?decision <${K}governs> ?module . ?module a ?type }`);
if (new Set(paths.map((r) => r.decision)).size !== 4) throw new Error("vision must reach four decisions and real typed code witnesses");
const episodes = ask(`SELECT ?episode ?source WHERE { ?episode a <${K}Episode> ; <http://www.w3.org/ns/prov#wasDerivedFrom> ?source }`);
if (episodes.length !== 4 || episodes.some((r) => !r.source.startsWith("https://github.com/scbrown/quipu/blob/main/"))) {
  throw new Error("four source-backed episodes required");
}
console.log(`Contributor receiver proof: ${paths.length} vision→decision→typed code paths, ${episodes.length} source-backed episodes`);
