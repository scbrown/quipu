//! Bounded directory and HTTP transport for portable text shares.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::Component;
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
    Ok(ShareImportRequest {
        manifest,
        export_ntriples: get("export.nt")?,
        shapes_turtle: get("shapes.ttl")?,
        source: source.to_string(),
        actor: None,
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

/// Reads a portable share directory or `.qpack[.tar.gz]` without opening a store.
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
pub fn import_in_memory(
    reference: &str,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<(crate::Store, crate::share_import::ShareImportResult)> {
    let mut request = if reference.starts_with("https://") || reference.starts_with("http://") {
        #[cfg(feature = "remote")]
        {
            read_url(reference)?
        }
        #[cfg(not(feature = "remote"))]
        {
            return Err(Error::InvalidValue(
                "URL share import requires the remote feature".into(),
            ));
        }
    } else {
        read_local(reference)?
    };
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
    }
}
