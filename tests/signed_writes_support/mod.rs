//! Shared harness for the signed-write server tests: a real `quipu-server`
//! per test, raw HTTP, and a client that signs v1 or v2 requests.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine as _;
#[allow(unused_imports)] // each test binary uses a different subset
pub use quipu::session_attestation::{
    AttestationEnvelope, SessionBinding, SignedBinding, WRITE_V1, WRITE_V2, WriteBinding,
    body_sha256, canonical_message,
};
pub use ring::signature::{Ed25519KeyPair, KeyPair};

pub const BEARER: &str = "signed-writes-test-bearer";

/// Every test here spawns a server on a port reserved-then-released, so two
/// concurrent tests can race for one port and a readiness probe can reach the
/// OTHER test's server. Measured: the no-op arm failed with an empty reply
/// under the parallel runner and passed alone. Serialize; each arm is ~0.4 s.
pub static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
pub const AGENT: &str = "urn:crew:seeds-test";
pub const INTRODUCER: &str = "urn:crew:lead";

pub struct Server {
    pub child: Child,
    pub address: std::net::SocketAddr,
    pub root: std::path::PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn spawn(root: &std::path::Path) -> Server {
    std::fs::create_dir_all(root.join(".bobbin")).unwrap();
    std::fs::write(
        root.join(".bobbin/config.toml"),
        format!("[quipu.server]\nauth_token = '{BEARER}'\n"),
    )
    .unwrap();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let log = std::fs::File::create(root.join("server.log")).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_quipu-server"))
        .args(["--bind", &address.to_string(), "--db"])
        .arg(root.join("store.db"))
        .current_dir(root)
        .env_clear()
        .env("HOME", root)
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    let mut server = Server {
        child,
        address,
        root: root.to_path_buf(),
    };
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until {
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "server exited: {}",
            std::fs::read_to_string(root.join("server.log")).unwrap()
        );
        if matches!(server.send("GET", "/health", &[], ""), Ok((200, _))) {
            return server;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("server did not become healthy");
}

impl Server {
    /// One raw HTTP request; returns (status, body).
    pub fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, String)],
        body: &str,
    ) -> std::io::Result<(u16, String)> {
        let mut socket = TcpStream::connect_timeout(&self.address, Duration::from_secs(1))?;
        socket.set_read_timeout(Some(Duration::from_secs(10)))?;
        let extra: String = headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect();
        write!(
            socket,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )?;
        let mut reply = String::new();
        socket.read_to_string(&mut reply)?;
        let Some(status) = reply.split_whitespace().nth(1).and_then(|s| s.parse().ok()) else {
            panic!(
                "{method} {path}: no HTTP status in reply {reply:?}; server log:\n{}",
                std::fs::read_to_string(self.root.join("server.log")).unwrap_or_default()
            );
        };
        let payload = reply
            .split_once("\r\n\r\n")
            .map_or("", |(_, b)| b)
            .to_owned();
        Ok((status, payload))
    }

    pub fn bearer(&self, path: &str, body: &str) -> (u16, String) {
        self.send(
            "POST",
            path,
            &[
                ("Authorization", format!("Bearer {BEARER}")),
                ("Content-Type", "application/json".into()),
            ],
            body,
        )
        .unwrap()
    }

    pub fn count(&self, subject: &str) -> usize {
        let query = serde_json::json!({
            "query": format!("SELECT ?p ?o WHERE {{ <{subject}> ?p ?o }}")
        })
        .to_string();
        let (status, body) = self
            .send(
                "POST",
                "/query",
                &[("Content-Type", "application/json".into())],
                &query,
            )
            .unwrap();
        assert_eq!(status, 200, "{body}");
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        json["rows"].as_array().map_or(0, Vec::len)
    }
}

pub struct Client {
    pub key: Ed25519KeyPair,
    pub binding: SessionBinding,
    pub nonce: u128,
}

impl Client {
    pub fn new(session: &str, seed: u8) -> Self {
        let key = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
        let now = quipu::time::epoch_secs();
        let binding = SessionBinding::new(
            AGENT,
            session,
            hex::encode(key.public_key().as_ref()),
            INTRODUCER,
            now - 60,
            now + 3600,
        )
        .unwrap();
        Self {
            key,
            // Write-granted: registration is share-only unless granted
            // (aegis-bys8d1); the share-only arm below clears this.
            binding: SessionBinding {
                allow_write: true,
                ..binding
            },
            nonce: u128::from(seed) << 64,
        }
    }

    /// Headers for a signed POST of `body` to `path`, with a FRESH nonce.
    pub fn sign(&mut self, path: &str, body: &str) -> Vec<(&'static str, String)> {
        self.nonce += 1;
        self.sign_with(path, body, &format!("{:032x}", self.nonce))
    }

    pub fn sign_with(&self, path: &str, body: &str, nonce: &str) -> Vec<(&'static str, String)> {
        let mut envelope = AttestationEnvelope {
            version: WRITE_V1.into(),
            key_id: self.binding.key_id.clone(),
            session: self.binding.session.clone(),
            introducer: INTRODUCER.into(),
            issued_at_epoch: quipu::time::epoch_secs(),
            nonce: nonce.into(),
            signature: String::new(),
            audience: None,
        };
        let hash = body_sha256(body.as_bytes());
        let write = WriteBinding {
            method: "POST",
            path,
            content_type: "application/json",
            body_sha256: &hash,
            audience: None,
        };
        let message = canonical_message(&envelope, &SignedBinding::Write(write));
        envelope.signature = hex::encode(self.key.sign(&message).as_ref());
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&envelope).unwrap());
        vec![
            ("x-quipu-attestation", header),
            ("Content-Type", "application/json".into()),
        ]
    }
}

impl Client {
    /// A `quipu-write-v2` request: the message signs `signed_for`, and the
    /// envelope CLAIMS `claimed`. They differ only in the swap arm.
    pub fn sign_v2(
        &mut self,
        path: &str,
        body: &str,
        signed_for: Option<&str>,
        claimed: Option<&str>,
        version: &str,
    ) -> Vec<(&'static str, String)> {
        self.nonce += 1;
        let mut envelope = AttestationEnvelope {
            version: version.into(),
            key_id: self.binding.key_id.clone(),
            session: self.binding.session.clone(),
            introducer: INTRODUCER.into(),
            issued_at_epoch: quipu::time::epoch_secs(),
            nonce: format!("{:032x}", self.nonce),
            signature: String::new(),
            audience: signed_for.map(str::to_owned),
        };
        let hash = body_sha256(body.as_bytes());
        let write = WriteBinding {
            method: "POST",
            path,
            content_type: "application/json",
            body_sha256: &hash,
            audience: signed_for,
        };
        let message = canonical_message(&envelope, &SignedBinding::Write(write));
        envelope.signature = hex::encode(self.key.sign(&message).as_ref());
        envelope.audience = claimed.map(str::to_owned);
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&envelope).unwrap());
        vec![
            ("x-quipu-attestation", header),
            ("Content-Type", "application/json".into()),
        ]
    }
}

pub fn store_id(dir: &tempfile::TempDir) -> String {
    quipu::Store::open(dir.path().join("store.db").to_str().unwrap())
        .unwrap()
        .store_id()
        .unwrap()
}

pub fn knot(n: u32) -> (String, String) {
    let subject = format!("urn:signed:probe{n}");
    let body =
        serde_json::json!({ "turtle": format!("<{subject}> <urn:label> \"p{n}\" .") }).to_string();
    (subject, body)
}

/// A store with `clients` registered (out of band, as `quipu attest` does).
pub fn fixture(clients: &[&Client]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let store = quipu::Store::open(dir.path().join("store.db").to_str().unwrap()).unwrap();
    for client in clients {
        store.attestation_register(&client.binding).unwrap();
    }
    dir
}

pub fn verdict(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body).unwrap()["verdict"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}
