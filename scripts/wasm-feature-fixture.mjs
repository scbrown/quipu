import { execFileSync } from "node:child_process";
import { writeFileSync, readFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

// A native-produced pack with strict shapes, adopted Datalog rules, and two
// distinct semantic documents. Nothing is copied from an installed store.
export function featureFixture(repo, binary, work) {
  mkdirSync(work, { recursive: true });
  const db = join(work, "fixture.db");
  const shapes = join(work, "shapes.ttl");
  const seed = join(work, "seed.ttl");
  writeFileSync(shapes, `@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .
@prefix rule: <http://quipu.local/rule#> .
ex:WidgetShape a sh:NodeShape ; sh:targetClass ex:Widget ;
  sh:property [ sh:path rdfs:label ; sh:minCount 1 ] ;
  sh:property [ sh:path ex:required ; sh:minCount 1 ] .
ex:backRule a rule:Rule ; rule:id "browser-back" ;
  rule:prefix "http://example.org/" ;
  rule:head "derivedBack(?y, ?x)" ; rule:body "connects(?x, ?y)" .
`);
  writeFileSync(seed, `@prefix ex: <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:cat a ex:Widget ; ex:required true ; rdfs:label "Domestic feline" ;
  rdfs:comment "A furry kitten purrs and chases mice" .
ex:car a ex:Widget ; ex:required true ; rdfs:label "Automobile" ;
  rdfs:comment "A motor vehicle with wheels drives on roads" .
`);
  const run = (args) => execFileSync(binary, args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  run(["shapes", "load", "fixture", shapes, "--db", db]);
  run(["knot", seed, "--db", db]);
  const policyShapes = join(work, "strict-policy-shapes.ttl");
  writeFileSync(policyShapes, readFileSync(join(repo, "examples/sharing-demo/policy-shapes.ttl"), "utf8") + `
<urn:demo:IdentifierPolicyShape>
  sh:property [ sh:path aegis:regex ; sh:minCount 1 ] ;
  sh:property [ sh:path aegis:enforcementTier ; sh:minCount 1 ] .
`);
  run(["shapes", "load", "policy", policyShapes, "--db", db]);
  run(["knot", join(repo, "examples/sharing-demo/policy.ttl"), "--graph", "urn:fixture:policy", "--db", db]);
  const share = join(work, "share");
  run(["share", "--output", share, "--db", db]);
  const pack = join(work, "fixture.qpack.tar.gz");
  execFileSync("tar", ["-C", share, "-czf", pack, "."]);
  return { pack, shapes };
}
