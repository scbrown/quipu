//! Operator-only native configuration. None of these methods has a transport
//! caller. A future enrollment/curator adapter must independently establish
//! identity and authority before invoking them; an author string is no proof.

use super::*;

impl Store {
    pub(in crate::store) fn migrate_decision_deck(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS deck_config (
               id INTEGER PRIMARY KEY CHECK(id=1), data BLOB NOT NULL,
               enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)));
             CREATE TABLE IF NOT EXISTS deck_credentials (
               id TEXT PRIMARY KEY, data BLOB NOT NULL,
               revoked INTEGER NOT NULL DEFAULT 0, sign_count INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS deck_requests (
               id TEXT PRIMARY KEY, revision INTEGER NOT NULL, data BLOB NOT NULL);
             CREATE TABLE IF NOT EXISTS deck_challenges (
               nonce TEXT PRIMARY KEY, data BLOB NOT NULL, consumed INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS deck_receipts (
               nonce TEXT PRIMARY KEY, request TEXT NOT NULL, revision INTEGER NOT NULL,
               data BLOB NOT NULL, tx INTEGER, UNIQUE(request,revision));",
        )?;
        Ok(())
    }

    /// Install application scope once through trusted database-file access.
    /// Starts disabled. This is not an enrollment or TLS-trust ceremony.
    pub fn deck_configure(&mut self, config: &Config) -> Result<()> {
        let expected = format!("https://{}", config.rp_id);
        if config.origin != expected
            || config.actor.is_empty()
            || config.author.is_empty()
            || !config.rp_id.contains('.')
            || config
                .rp_id
                .bytes()
                .any(|b| !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'))
            || config
                .rp_id
                .split('.')
                .any(|s| s.is_empty() || s.starts_with('-') || s.ends_with('-'))
        {
            return Err(refuse(
                "requires exact dedicated HTTPS origin/RP and explicit identities",
            ));
        }
        self.conn.execute(
            "INSERT INTO deck_config(id,data) VALUES(1,?1)",
            [encode(config)?],
        )?;
        Ok(())
    }

    /// Native operator switch. No browser, graph or carried-share enable path.
    pub fn deck_set_enabled(&mut self, enabled: bool) -> Result<()> {
        if self
            .conn
            .execute("UPDATE deck_config SET enabled=?1 WHERE id=1", [enabled])?
            != 1
        {
            return Err(refuse("scope is not configured"));
        }
        Ok(())
    }

    pub(super) fn deck_scope(&self, require_enabled: bool) -> Result<Config> {
        let row: Option<(Vec<u8>, bool)> = self
            .conn
            .query_row("SELECT data,enabled FROM deck_config WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let (data, enabled) = row.ok_or_else(|| refuse("disabled: no operator scope"))?;
        if require_enabled && !enabled {
            return Err(refuse("disabled"));
        }
        decode(&data)
    }

    /// Enroll a public credential after an independent operator ceremony.
    /// Never replace or resurrect a retained ID; rotation requires a new ID.
    pub fn deck_enroll(&mut self, credential: &Credential) -> Result<()> {
        self.deck_scope(false)?;
        let id = crate::verdict_schemes::b64url_decode(&credential.id)
            .map_err(|_| refuse("credential ID encoding"))?;
        use base64::Engine as _;
        if id.is_empty()
            || id.len() > 1024
            || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&id) != credential.id
            || credential.expires_at <= now()?
        {
            return Err(refuse("invalid credential ID or expiry"));
        }
        let key = crate::verdict_schemes::cose::parse(&credential.cose_key)
            .map_err(|_| refuse("invalid COSE key"))?;
        match (
            Scheme::parse(&credential.scheme).map_err(|_| refuse("unsupported scheme"))?,
            key,
        ) {
            (Scheme::WebauthnEs256, crate::verdict_schemes::cose::CoseKey::Es256 { .. })
            | (Scheme::WebauthnEddsa, crate::verdict_schemes::cose::CoseKey::Ed25519 { .. }) => {}
            _ => return Err(refuse("scheme/key mismatch")),
        }
        self.conn.execute(
            "INSERT INTO deck_credentials(id,data) VALUES(?1,?2)",
            params![credential.id, encode(credential)?],
        )?;
        Ok(())
    }

    /// Revoke permanently, retaining old receipts and the public enrollment.
    pub fn deck_revoke(&mut self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("UPDATE deck_credentials SET revoked=1 WHERE id=?1", [id])?
            == 1)
    }

    /// Read-only operator view; `None` when no scope was ever configured.
    pub fn deck_status(&self) -> Result<Option<DeckStatus>> {
        let row: Option<(Vec<u8>, bool)> = self
            .conn
            .query_row("SELECT data,enabled FROM deck_config WHERE id=1", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((data, enabled)) = row else {
            return Ok(None);
        };
        let config: Config = decode(&data)?;
        let mut statement = self
            .conn
            .prepare("SELECT data,revoked,sign_count FROM deck_credentials ORDER BY rowid")?;
        let credentials = statement
            .query_map([], |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(data, revoked, sign_count)| {
                let credential: Credential = decode(&data)?;
                Ok(CredentialStatus {
                    fingerprint: credential_fingerprint(&credential.cose_key),
                    id: credential.id,
                    scheme: credential.scheme,
                    expires_at: credential.expires_at,
                    revoked,
                    sign_count,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(DeckStatus {
            origin: config.origin,
            rp_id: config.rp_id,
            actor: config.actor,
            author: config.author,
            enabled,
            credentials,
        }))
    }

    /// Author one explicit request through trusted operator access. Transport
    /// callers must not expose this method with a self-asserted author string.
    /// Revisions increase strictly; a challenge for an older revision fails.
    pub fn deck_put_request(&mut self, request: &Request) -> Result<()> {
        let config = self.deck_scope(false)?;
        if request.author != config.author
            || request.revision == 0
            || request.expires_at <= now()?
            || request.policy.is_empty()
            || request.target.is_empty()
            || request.summary.is_empty()
            || request.choices.is_empty()
            || request.choices.len() > 3
            || request
                .choices
                .iter()
                .any(|c| !["approve", "reject", "changes"].contains(&c.as_str()))
            || request
                .choices
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != request.choices.len()
            || encode(request)?.len() > 32768
            || oxrdf::NamedNode::new(request.id.clone()).is_err()
            || request.id.starts_with("urn:quipu:decision:")
        {
            return Err(refuse("invalid request or unauthorized curator"));
        }
        self.deck_atomic(&config.author, |store, ts| {
            let previous: Option<u32> = store.conn.query_row("SELECT revision FROM deck_requests WHERE id=?1", [&request.id], |r| r.get(0)).optional()?;
            if previous.is_some_and(|old| request.revision <= old) {
                return Err(refuse("request revision must increase"));
            }
            let graph = store.intern(GRAPH)?;
            let entity = store.intern(&request.id)?;
            let mut datums: Vec<Datum> = store.current_facts_in_graph(graph)?.into_iter().filter(|f| f.entity == entity).map(|f| Datum { entity:f.entity,attribute:f.attribute,value:f.value,valid_from:ts.into(),valid_to:None,op:Op::Retract }).collect();
            datums.push(typed(store,&request.id,"DecisionRequest",ts)?);
            for (field,value) in [
                ("forPolicy", Value::Str(request.policy.clone())),
                ("forTarget", Value::Str(request.target.clone())),
                ("expiresAt", Value::Int(request.expires_at)),
                ("evidenceHash", Value::Str(crate::share::sha256(&encode(request)?))),
                ("deckRequest", Value::Str(String::from_utf8(encode(request)?).map_err(|_| refuse("request encoding"))?)),
            ] { datums.push(datum(store,&request.id,field,value,ts)?); }
            store.conn.execute("INSERT INTO deck_requests(id,revision,data) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,data=excluded.data", params![request.id,request.revision,encode(request)?])?;
            Ok(((),datums))
        })
    }

    pub(super) fn deck_request(&self, id: &str) -> Result<Request> {
        let data: Vec<u8> = self
            .conn
            .query_row("SELECT data FROM deck_requests WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| refuse("unknown request"))?;
        decode(&data)
    }

    pub(super) fn deck_credential(&self, id: &str) -> Result<(Credential, u32)> {
        let row: Option<(Vec<u8>, bool, u32)> = self
            .conn
            .query_row(
                "SELECT data,revoked,sign_count FROM deck_credentials WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (data, revoked, count) = row.ok_or_else(|| refuse("unknown credential"))?;
        let credential: Credential = decode(&data)?;
        if revoked || credential.expires_at <= now()? {
            return Err(refuse("revoked or expired credential"));
        }
        Ok((credential, count))
    }
}
