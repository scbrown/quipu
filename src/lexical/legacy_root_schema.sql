CREATE VIRTUAL TABLE IF NOT EXISTS lexical_fts USING fts5(
    label, alt_label, description, attributes, type_names, iri_tokens,
    entity_iri UNINDEXED, type_iri UNINDEXED, language UNINDEXED,
    datatype UNINDEXED, graph_id UNINDEXED,
    tokenize = 'unicode61'
);
CREATE TABLE IF NOT EXISTS lexical_progress (
    id INTEGER PRIMARY KEY CHECK(id=1), cursor INTEGER NOT NULL,
    highwater INTEGER NOT NULL, complete INTEGER NOT NULL,
    documents INTEGER NOT NULL DEFAULT 0 CHECK(documents>=0)
);
INSERT OR IGNORE INTO lexical_progress
    SELECT 1, 0, highwater, highwater=0, 0
    FROM (SELECT coalesce(max(rowid),0) AS highwater FROM facts);
CREATE VIEW IF NOT EXISTS lexical_source AS
SELECT f.rowid AS fact_id,
    CASE WHEN p.iri IN ('http://www.w3.org/2000/01/rdf-schema#label',
                       'http://www.w3.org/2004/02/skos/core#prefLabel')
         THEN quipu_lexical_text(f.v) ELSE '' END AS label,
    CASE WHEN p.iri='http://www.w3.org/2004/02/skos/core#altLabel'
         THEN quipu_lexical_text(f.v) ELSE '' END AS alt_label,
    CASE WHEN p.iri IN ('http://www.w3.org/2000/01/rdf-schema#comment',
                       'http://www.w3.org/2004/02/skos/core#definition')
         THEN quipu_lexical_text(f.v) ELSE '' END AS description,
    CASE WHEN p.iri NOT IN ('http://www.w3.org/2000/01/rdf-schema#label',
              'http://www.w3.org/2004/02/skos/core#prefLabel',
              'http://www.w3.org/2004/02/skos/core#altLabel',
              'http://www.w3.org/2000/01/rdf-schema#comment',
              'http://www.w3.org/2004/02/skos/core#definition')
         THEN quipu_lexical_text(f.v) ELSE '' END AS attributes,
    CASE WHEN p.iri='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
         THEN quipu_lexical_local(coalesce(o.iri,'')) ELSE '' END AS type_names,
    quipu_lexical_local(e.iri) AS iri_tokens,
    e.iri AS entity_iri,
    CASE WHEN p.iri='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
         THEN o.iri ELSE NULL END AS type_iri,
    quipu_lexical_language(f.v) AS language,
    quipu_lexical_datatype(f.v) AS datatype,
    f.g AS graph_id
FROM facts f NOT INDEXED JOIN terms e ON e.id=f.e JOIN terms p ON p.id=f.a
LEFT JOIN terms o ON o.id=quipu_lexical_ref(f.v)
WHERE f.op=1 AND f.g=0;
CREATE TRIGGER IF NOT EXISTS lexical_insert AFTER INSERT ON facts BEGIN
    UPDATE lexical_progress SET documents=documents+1 WHERE id=1
        AND NEW.op=1 AND NEW.g=0 AND NOT EXISTS(SELECT 1 FROM lexical_fts WHERE rowid=NEW.rowid);
    INSERT OR REPLACE INTO lexical_fts(rowid,label,alt_label,description,attributes,type_names,iri_tokens,entity_iri,type_iri,language,datatype,graph_id)
    SELECT fact_id,label,alt_label,description,attributes,type_names,iri_tokens,entity_iri,type_iri,language,datatype,graph_id
    FROM lexical_source WHERE fact_id=NEW.rowid;
END;
CREATE TRIGGER IF NOT EXISTS lexical_delete AFTER DELETE ON facts BEGIN
    UPDATE lexical_progress SET documents=documents-1 WHERE id=1
        AND EXISTS(SELECT 1 FROM lexical_fts WHERE rowid=OLD.rowid);
    DELETE FROM lexical_fts WHERE rowid=OLD.rowid;
END;
CREATE TRIGGER IF NOT EXISTS lexical_update AFTER UPDATE OF e,a,v,g,op ON facts BEGIN
    UPDATE lexical_progress SET documents=documents-1 WHERE id=1
        AND EXISTS(SELECT 1 FROM lexical_fts WHERE rowid=OLD.rowid);
    DELETE FROM lexical_fts WHERE rowid=OLD.rowid;
    UPDATE lexical_progress SET documents=documents+1 WHERE id=1 AND NEW.op=1 AND NEW.g=0;
    INSERT OR REPLACE INTO lexical_fts(rowid,label,alt_label,description,attributes,type_names,iri_tokens,entity_iri,type_iri,language,datatype,graph_id)
    SELECT fact_id,label,alt_label,description,attributes,type_names,iri_tokens,entity_iri,type_iri,language,datatype,graph_id
    FROM lexical_source WHERE fact_id=NEW.rowid;
END;
