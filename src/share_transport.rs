//! Bounded directory and HTTP transport for portable text shares.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::Component;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

use crate::error::{Error, Result};
use crate::share::ShareManifest;
use crate::share_import::ShareImportRequest;

/// Maximum compressed response accepted by the CLI importer.
pub const MAX_SHARE_DOWNLOAD_BYTES: usize = 16 * 1024 * 1024;
/// Maximum total payload bytes expanded from an archive.
pub const MAX_SHARE_EXPANDED_BYTES: usize = 64 * 1024 * 1024;

const REQUIRED: [&str; 3] = ["manifest.json", "export.nt", "shapes.ttl"];

fn request_from_files(
    files: &BTreeMap<String, String>,
    source: &str,
) -> Result<ShareImportRequest> {
    let get = |name: &str| {
        files
            .get(name)
            .cloned()
            .ok_or_else(|| Error::InvalidValue(format!("share transport: missing {name}")))
    };
    let manifest = serde_json::from_str::<ShareManifest>(&get("manifest.json")?)
        .map_err(|e| Error::InvalidValue(format!("share manifest: {e}")))?;
    // LIFT THE EMBEDDED ENVELOPE (aegis-tadzdf). A share minted with `--attest`
    // carries its envelope in the manifest; if we do not lift it here the
    // attestation is inert and `claimed` is unreachable from the CLI -- the same
    // "capability nothing invokes" defect this bead exists to close, one layer
    // down. Measured: the first end-to-end run of the exercise reported
    // `transport` for a share that carried a valid attestation.
    //
    // This grants NO trust. It only supplies what the share already brought;
    // whether that reaches `claimed` or `attested` is decided in
    // `share_attestation` by whether the session is REGISTERED here, which
    // importing cannot change.
    let embedded = manifest.attestation.as_ref().map(|a| a.envelope.clone());
    Ok(ShareImportRequest {
        #[cfg(not(target_arch = "wasm32"))]
        attestation: embedded,
        manifest,
        export_ntriples: get("export.nt")?,
        shapes_turtle: get("shapes.ttl")?,
        source: source.to_string(),
        actor: None,
        accept_exact: false,
    })
}

fn archive_files<R: Read>(reader: R, source: &str) -> Result<BTreeMap<String, String>> {
    let mut archive = tar::Archive::new(reader);
    let mut files = BTreeMap::new();
    let mut expanded = 0_usize;
    let entries = archive
        .entries()
        .map_err(|e| Error::InvalidValue(format!("share archive {source}: {e}")))?;
    for entry in entries {
        let mut entry =
            entry.map_err(|e| Error::InvalidValue(format!("share archive {source}: {e}")))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| Error::InvalidValue(format!("share archive path: {e}")))?
            .into_owned();
        if path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(Error::InvalidValue(format!(
                "share archive contains unsafe path: {}",
                path.display()
            )));
        }
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| Error::InvalidValue("share archive path is not UTF-8".into()))?
            .to_string();
        if !REQUIRED.contains(&name.as_str()) && name != "export.ttl" && name != "manifest.ttl" {
            return Err(Error::InvalidValue(format!(
                "share archive contains undeclared file: {}",
                path.display()
            )));
        }
        if files.contains_key(&name) {
            return Err(Error::InvalidValue(format!(
                "share archive contains duplicate {name}"
            )));
        }
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take((MAX_SHARE_EXPANDED_BYTES - expanded + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::InvalidValue(format!("share archive {name}: {e}")))?;
        expanded = expanded.saturating_add(bytes.len());
        if expanded > MAX_SHARE_EXPANDED_BYTES {
            return Err(Error::InvalidValue(format!(
                "share archive exceeds {MAX_SHARE_EXPANDED_BYTES} expanded bytes"
            )));
        }
        let text = String::from_utf8(bytes)
            .map_err(|e| Error::InvalidValue(format!("share archive {name} UTF-8: {e}")))?;
        files.insert(name, text);
    }
    Ok(files)
}

/// Reads a `.qpack` archive already held in memory.
///
/// The same bounded expansion and undeclared-path rejection [`read_local`]
/// applies, on bytes the caller obtained however it liked — a browser `fetch`,
/// a file picker, an embedded asset. `gzip` selects whether the bytes are
/// gzip-compressed (`.tar.gz`) or a bare tar.
///
/// # Errors
/// The archive is malformed, exceeds [`MAX_SHARE_EXPANDED_BYTES`] expanded,
/// carries an undeclared or unsafe path, or is missing a required payload.
pub fn read_archive_bytes(bytes: &[u8], source: &str, gzip: bool) -> Result<ShareImportRequest> {
    if bytes.len() > MAX_SHARE_DOWNLOAD_BYTES {
        return Err(Error::InvalidValue(format!(
            "share archive exceeds {MAX_SHARE_DOWNLOAD_BYTES} bytes"
        )));
    }
    let files = if gzip {
        archive_files(flate2::read::GzDecoder::new(Cursor::new(bytes)), source)?
    } else {
        archive_files(Cursor::new(bytes), source)?
    };
    request_from_files(&files, source)
}

/// Reads a portable share directory or `.qpack[.tar.gz]` without opening a store.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_local(reference: &str) -> Result<ShareImportRequest> {
    let path = Path::new(reference);
    if path.is_dir() {
        let mut files = BTreeMap::new();
        for name in REQUIRED {
            let bytes = std::fs::read(path.join(name))
                .map_err(|e| Error::InvalidValue(format!("share transport {name}: {e}")))?;
            files.insert(
                name.to_string(),
                String::from_utf8(bytes)
                    .map_err(|e| Error::InvalidValue(format!("share transport {name}: {e}")))?,
            );
        }
        return request_from_files(&files, reference);
    }
    let bytes = std::fs::read(path)
        .map_err(|e| Error::InvalidValue(format!("share transport {reference}: {e}")))?;
    let files = if reference.ends_with(".gz") {
        archive_files(flate2::read::GzDecoder::new(Cursor::new(bytes)), reference)?
    } else {
        archive_files(Cursor::new(bytes), reference)?
    };
    request_from_files(&files, reference)
}

/// Reads a share from a local directory/archive or an HTTP(S) reference.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_reference(reference: &str) -> Result<ShareImportRequest> {
    if reference.starts_with("https://") || reference.starts_with("http://") {
        #[cfg(feature = "remote")]
        {
            return read_url(reference);
        }
        #[cfg(not(feature = "remote"))]
        {
            return Err(Error::InvalidValue(
                "URL share import requires the remote feature".into(),
            ));
        }
    }
    read_local(reference)
}

#[cfg(feature = "remote")]
fn fetch(url: &str) -> Result<Vec<u8>> {
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(Error::InvalidValue(
            "share URL must use http or https".into(),
        ));
    }
    let response = ureq::get(url)
        .set("Accept", "application/gzip, application/x-tar, application/json, text/turtle, application/n-triples")
        .call()
        .map_err(|e| Error::InvalidValue(format!("share fetch {url}: {e}")))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((MAX_SHARE_DOWNLOAD_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::InvalidValue(format!("share fetch {url}: {e}")))?;
    if bytes.len() > MAX_SHARE_DOWNLOAD_BYTES {
        return Err(Error::InvalidValue(format!(
            "share response exceeds {MAX_SHARE_DOWNLOAD_BYTES} bytes"
        )));
    }
    Ok(bytes)
}

/// Fetches a directory manifest or release artifact into bounded memory.
#[cfg(feature = "remote")]
pub fn read_url(url: &str) -> Result<ShareImportRequest> {
    if url.ends_with('/') {
        let mut files = BTreeMap::new();
        for name in REQUIRED {
            let bytes = fetch(&format!("{url}{name}"))?;
            files.insert(
                name.to_string(),
                String::from_utf8(bytes)
                    .map_err(|e| Error::InvalidValue(format!("share fetch {name} UTF-8: {e}")))?,
            );
        }
        return request_from_files(&files, url);
    }
    let bytes = fetch(url)?;
    let files = if url.ends_with(".gz") {
        archive_files(flate2::read::GzDecoder::new(Cursor::new(bytes)), url)?
    } else {
        archive_files(Cursor::new(bytes), url)?
    };
    request_from_files(&files, url)
}

/// Loads a directory, archive, or URL into a fresh in-memory store after verification.
#[cfg(not(target_arch = "wasm32"))]
pub fn import_in_memory(
    reference: &str,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<(crate::Store, crate::share_import::ShareImportResult)> {
    let mut request = read_reference(reference)?;
    request.actor = actor.map(str::to_string);
    let mut store = crate::Store::open_in_memory()?;
    let result = crate::share_import::import_share(&mut store, &request, timestamp, actor)?;
    Ok((store, result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::{ShareOptions, share};

    #[test]
    fn local_directory_loads_into_memory_without_a_database_file() {
        let temp = tempfile::tempdir().unwrap();
        let source = crate::Store::open_in_memory().unwrap();
        let out = temp.path().join("share");
        share(
            &source,
            out.to_str().unwrap(),
            &ShareOptions {
                no_shapes: true,
                ..ShareOptions::default()
            },
        )
        .unwrap();
        let (store, result) = import_in_memory(out.to_str().unwrap(), "2026-09-03", None).unwrap();
        assert_eq!(result.outcome, "staged");
        assert!(store.latest_tx_id().unwrap() > 0);
        assert_eq!(temp.path().read_dir().unwrap().count(), 1);
    }

    #[test]
    fn gzip_release_artifact_loads_the_same_share() {
        let temp = tempfile::tempdir().unwrap();
        let source = crate::Store::open_in_memory().unwrap();
        let out = temp.path().join("share");
        let manifest = share(
            &source,
            out.to_str().unwrap(),
            &ShareOptions {
                no_shapes: true,
                ..ShareOptions::default()
            },
        )
        .unwrap();
        let archive_path = temp.path().join("repository.qpack.tar.gz");
        let archive_file = std::fs::File::create(&archive_path).unwrap();
        let encoder = flate2::write::GzEncoder::new(archive_file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for name in REQUIRED {
            archive.append_path_with_name(out.join(name), name).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();

        let (_store, result) =
            import_in_memory(archive_path.to_str().unwrap(), "2026-09-03", None).unwrap();
        assert_eq!(result.share_id, manifest.share_id);

        // The browser holds the archive already — it fetched it — so it takes
        // the same bytes through `read_archive_bytes` instead of a path. Same
        // share, same id, no filesystem in the middle.
        let bytes = std::fs::read(&archive_path).unwrap();
        let from_bytes = read_archive_bytes(&bytes, "in-memory", true).unwrap();
        assert_eq!(from_bytes.manifest.share_id, manifest.share_id);
        assert_eq!(
            from_bytes.export_ntriples,
            std::fs::read_to_string(out.join("export.nt")).unwrap()
        );
    }

    // The bounds are what make it safe to hand this a network download, so they
    // are worth a test that FAILS if someone relaxes them. A truncated archive
    // is the cheap way to prove the reader is actually parsing rather than
    // accepting anything with a manifest-shaped name in it.
    #[test]
    fn read_archive_bytes_refuses_a_truncated_archive() {
        let temp = tempfile::tempdir().unwrap();
        let source = crate::Store::open_in_memory().unwrap();
        let out = temp.path().join("share");
        share(
            &source,
            out.to_str().unwrap(),
            &ShareOptions {
                no_shapes: true,
                ..ShareOptions::default()
            },
        )
        .unwrap();
        let archive_path = temp.path().join("t.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            std::fs::File::create(&archive_path).unwrap(),
            flate2::Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        for name in REQUIRED {
            archive.append_path_with_name(out.join(name), name).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();

        let bytes = std::fs::read(&archive_path).unwrap();
        // Control: the whole thing reads, so a failure below is the truncation
        // and not a broken fixture.
        assert!(read_archive_bytes(&bytes, "control", true).is_ok());
        assert!(read_archive_bytes(&bytes[..bytes.len() / 2], "truncated", true).is_err());
    }

    #[test]
    fn read_archive_bytes_refuses_an_undeclared_file() {
        let temp = tempfile::tempdir().unwrap();
        let source = crate::Store::open_in_memory().unwrap();
        let out = temp.path().join("share");
        share(
            &source,
            out.to_str().unwrap(),
            &ShareOptions {
                no_shapes: true,
                ..ShareOptions::default()
            },
        )
        .unwrap();
        std::fs::write(out.join("payload.sh"), "#!/bin/sh\necho no\n").unwrap();
        let archive_path = temp.path().join("t.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            std::fs::File::create(&archive_path).unwrap(),
            flate2::Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        for name in REQUIRED.iter().chain(["payload.sh"].iter()) {
            archive.append_path_with_name(out.join(name), name).unwrap();
        }
        archive.into_inner().unwrap().finish().unwrap();

        let bytes = std::fs::read(&archive_path).unwrap();
        let err = read_archive_bytes(&bytes, "hostile", true).unwrap_err();
        assert!(
            err.to_string().contains("undeclared file"),
            "expected an undeclared-file refusal, got: {err}"
        );
    }
}
