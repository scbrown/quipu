//! Exercise startup and write authorization through the actual server binary.
#![cfg(all(feature = "shacl", feature = "onnx", feature = "server"))]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Server {
    child: Child,
    address: std::net::SocketAddr,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(root: &std::path::Path) -> Server {
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
    Server { child, address }
}

fn request(server: &Server, token: Option<&str>, write: bool) -> std::io::Result<u16> {
    let mut socket = TcpStream::connect_timeout(&server.address, Duration::from_secs(1))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let body = if write {
        r#"{"turtle":"<urn:rotation:probe> <urn:label> \"proof\" ."}"#
    } else {
        ""
    };
    let auth = token.map_or_else(String::new, |token| {
        format!("Authorization: Bearer {token}\r\n")
    });
    let (method, path) = if write {
        ("POST", "/knot")
    } else {
        ("GET", "/health")
    };
    write!(
        socket,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )?;
    let mut reply = String::new();
    socket.read_to_string(&mut reply)?;
    Ok(reply.split_whitespace().nth(1).unwrap().parse().unwrap())
}

fn ready(server: &mut Server, root: &std::path::Path) {
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "server exited: {}",
            std::fs::read_to_string(root.join("server.log")).unwrap()
        );
        if request(server, None, false).ok() == Some(200) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("server did not become healthy");
}

fn configure(root: &std::path::Path, previous: Option<(&str, u64)>) {
    std::fs::create_dir_all(root.join(".bobbin")).unwrap();
    let mut config = "[quipu.server]\nauth_token = 'new-test-bearer'\n".to_string();
    if let Some((token, expiry)) = previous {
        config.push_str(&format!("previous_auth_token = '{token}'\nprevious_auth_token_expires_at_epoch_secs = {expiry}\n"));
    }
    std::fs::write(root.join(".bobbin/config.toml"), config).unwrap();
}

#[test]
fn expired_previous_bearer_warns_and_serves_new_writes_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    configure(dir.path(), Some(("old-test-bearer", 1)));
    for _ in 0..2 {
        let mut server = spawn(dir.path());
        ready(&mut server, dir.path());
        assert_eq!(
            request(&server, Some("new-test-bearer"), true).unwrap(),
            200
        );
        assert_eq!(
            request(&server, Some("old-test-bearer"), true).unwrap(),
            401
        );
        assert_eq!(request(&server, None, true).unwrap(), 401);
        assert!(server.child.try_wait().unwrap().is_none());
        let log = std::fs::read_to_string(dir.path().join("server.log")).unwrap();
        assert!(log.contains("expired previous bearer ignored"), "{log}");
        assert!(!log.contains("temporary previous bearer enabled"));
        for token in ["new-test-bearer", "old-test-bearer"] {
            assert!(!log.contains(token));
        }
    }
}

#[test]
fn both_bearers_write_during_grace_then_explicit_removal_invalidates_old() {
    let dir = tempfile::tempdir().unwrap();
    configure(
        dir.path(),
        Some(("old-test-bearer", quipu::time::epoch_secs() + 300)),
    );
    {
        let mut server = spawn(dir.path());
        ready(&mut server, dir.path());
        for token in ["new-test-bearer", "old-test-bearer"] {
            assert_eq!(request(&server, Some(token), true).unwrap(), 200);
        }
    }
    configure(dir.path(), None);
    let mut server = spawn(dir.path());
    ready(&mut server, dir.path());
    assert_eq!(
        request(&server, Some("new-test-bearer"), true).unwrap(),
        200
    );
    assert_eq!(
        request(&server, Some("old-test-bearer"), true).unwrap(),
        401
    );
}

#[test]
fn identical_bearers_still_exit_two_even_when_the_previous_deadline_expired() {
    let dir = tempfile::tempdir().unwrap();
    configure(dir.path(), Some(("new-test-bearer", 1)));
    let mut server = spawn(dir.path());
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(2));
            break;
        }
        assert!(Instant::now() < until, "contradictory config kept serving");
        std::thread::sleep(Duration::from_millis(20));
    }
    let log = std::fs::read_to_string(dir.path().join("server.log")).unwrap();
    assert!(log.contains("current and previous bearer must be distinct"));
    assert!(!log.contains("new-test-bearer"));
}
