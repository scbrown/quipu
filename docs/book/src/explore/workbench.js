// Presentation around the existing engine controls. All answers stay in the
// explorer's real query path; there are no mockup rows or simulated verdicts.
const $ = (selector) => document.querySelector(selector);
let report = null;

export function loadStarted(bytes) {
  report = null;
  for (const step of document.querySelectorAll("#load-cord li")) {
    step.classList.remove("done");
    step.querySelector("span").textContent = "Waiting";
  }
  stage("download", true, `${bytes.toLocaleString()} bytes`);
  $("#pack-chip").textContent = "Verifying the downloaded pack";
}

function stage(name, done, text) {
  const step = $(`#load-cord [data-step="${name}"]`);
  step.classList.toggle("done", done);
  step.querySelector("span").textContent = text;
}

export function loaded(value) {
  report = value;
  // loadQpack returns only after manifest/payload verification. Stage and
  // promotion have separate reported outcomes; don't turn a quarantine red
  // into a green "promoted" knot.
  stage("verify", true, "Manifest hashes checked");
  stage("stage", value.import.outcome === "staged",
    `${value.import.triples.quarantined.toLocaleString()} quarantined`);
  const promoted = value.promotion?.outcome === "promoted";
  stage("promote", promoted, promoted
    ? `${value.promotion.triples.toLocaleString()} triples`
    : "Not promoted");
  $("#build-chip").textContent = `v${window.__quipuBuild?.version ?? "unknown"}`;
  $("#pack-chip").textContent = promoted
    ? `${value.promotion.triples.toLocaleString()} triples · pack verified`
    : "Pack verified · promotion refused";
  $("#bad-write").disabled = !promoted;
  $("#validation-summary").textContent = value.shacl_compiled
    ? "Writes are checked against the loaded SHACL shapes. Try a refusal and inspect the report."
    : "This browser checks the loaded vocabulary. SHACL validation is unavailable in this engine build.";
  $("#bad-write").title = value.shacl_compiled
    ? "Attempt a real SHACL violation and verify nothing was written"
    : "This build lacks SHACL; the demonstration reports that limitation";
}

export function setupWorkbench() {
  const showTab = (name) => {
    for (const button of document.querySelectorAll("[data-tab]")) {
      const selected = button.dataset.tab === name;
      button.setAttribute("aria-selected", String(selected));
      button.tabIndex = selected ? 0 : -1;
      $(`#view-${button.dataset.tab}`).hidden = !selected;
    }
  };
  for (const button of document.querySelectorAll("[data-tab]")) {
    button.addEventListener("click", () => showTab(button.dataset.tab));
    button.addEventListener("keydown", (event) => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      const tabs = [...document.querySelectorAll("[data-tab]")];
      const index = tabs.indexOf(button);
      const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1
        : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
      showTab(tabs[next].dataset.tab);
      tabs[next].focus();
    });
  }
  showTab("ask");
  $("#sparql").readOnly = true;
  $("#edit-query").addEventListener("click", () => {
    $("#sparql").readOnly = !$("#sparql").readOnly;
    $("#edit-query").textContent = $("#sparql").readOnly ? "Edit query" : "Done editing";
    $("#sparql").focus();
  });
  $("#theme-choice").addEventListener("change", (event) => {
    if (event.target.value === "auto") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = event.target.value;
  });

  // Mirror the shipped starter controls rather than duplicating their queries
  // or taking sample answers from the visual mockup.
  const mirrorQuestions = () => {
    const target = $("#starter-questions");
    target.replaceChildren();
    for (const original of $("#canned").children) {
      const button = document.createElement("button");
      button.textContent = original.textContent;
      button.setAttribute("aria-pressed", "false");
      button.addEventListener("click", () => {
        showTab("ask");
        for (const other of target.children) other.setAttribute("aria-pressed", "false");
        button.setAttribute("aria-pressed", "true");
        $("#bad-verdict").hidden = true;
        original.click();
      });
      target.append(button);
    }
  };
  new MutationObserver(mirrorQuestions).observe($("#canned"), { childList: true });

  // Existing "show query" links and selected-file inspectors still navigate
  // to their actual query, including when their original panel is tabbed away.
  for (const id of ["types-showq", "browse-showq", "detail-showq", "facts-showq", "graph-showq"]) {
    $(`#${id}`).addEventListener("click", () => showTab("ask"));
  }

  $("#bad-write").addEventListener("click", async () => {
    showTab("ask");
    const out = $("#bad-verdict");
    out.hidden = false;
    out.replaceChildren();
    const heading = document.createElement("h3");
    out.append(heading);
    if (!report?.shacl_compiled) {
      heading.textContent = "SHACL demonstration unavailable in this engine build";
      out.append(document.createTextNode(
        "The current browser can check vocabulary, but that is not SHACL validation. "
        + "This control requires the full-feature engine before it can demonstrate a shape refusal.",
      ));
      return;
    }
    const button = $("#bad-write");
    button.disabled = true;
    const name = `explorer-shacl-probe-${crypto.randomUUID()}`;
    // Publication policy shapes require regex and a block tier. The deliberate
    // omission must be rejected by SHACL, not by an unknown-type front door.
    const probe = `SELECT ?s WHERE { ?s <http://www.w3.org/2000/01/rdf-schema#label> "${name}" }`;
    const query = document.createElement("pre"); query.textContent = probe; out.append(query);
    try {
      const before = await window.quipu.query(probe);
      if (!Array.isArray(before.rows) || before.rows.length !== 0) throw new Error("Probe is not fresh");
      let refusal = null;
      try {
        await window.quipu.episode({
          name, source: "interactive SHACL refusal demonstration",
          nodes: [{ name, type: "InternalIdentifierPattern" }], edges: [],
        });
      } catch (error) { refusal = error.message; }
      const after = await window.quipu.query(probe);
      const positive = await window.quipu.query("SELECT ?s WHERE { ?s ?p ?o } LIMIT 1");
      if (!refusal || !/shacl|shape|validation/i.test(refusal)
        || !Array.isArray(after.rows) || after.rows.length !== 0 || !positive.rows?.length) {
        throw new Error("The engine did not establish SHACL refusal plus absence behind a positive control");
      }
      heading.textContent = "Refused by the loaded SHACL shapes";
      const feedback = document.createElement("pre"); feedback.textContent = refusal;
      out.append(feedback, document.createTextNode("Follow-up query: 0 rows written. Existing graph control: 1 row."));
    } catch (error) {
      heading.textContent = "Demonstration could not establish the promised result";
      out.append(document.createTextNode(error.message));
    } finally { button.disabled = false; }
  });
}
