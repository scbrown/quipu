//! Git plumbing: explicit immutable snapshots, never file-order assumptions.
use crate::error::Result;
use crate::git_merge::{Decisions, Merged, Pack, invalid, io, plan, spawn_git, validate};
use crate::share::{manifest_bytes, manifest_turtle};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let o = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| spawn_git(&e))?;
    if !o.status.success() {
        return Err(invalid(String::from_utf8_lossy(&o.stderr).into_owned()));
    }
    Ok(o.stdout)
}
fn text(repo: &Path, args: &[&str]) -> Result<String> {
    String::from_utf8(git(repo, args)?).map_err(io)
}
fn commit(repo: &Path, r: &str) -> Result<String> {
    Ok(text(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{r}^{{commit}}"),
        ],
    )?
    .trim()
    .into())
}
fn file(repo: &Path, r: &str, path: &str) -> Result<Option<String>> {
    let entry = text(repo, &["ls-tree", r, "--", path])?;
    if entry.is_empty() {
        return Ok(None);
    }
    if !entry.starts_with("100644 ") && !entry.starts_with("100755 ") {
        return Err(invalid("qpack payload must be a regular Git blob"));
    }
    text(repo, &["show", &format!("{r}:{path}")]).map(Some)
}
fn pack(repo: &Path, r: &str, dir: &str) -> Result<Option<Pack>> {
    let read = |name: &str| file(repo, r, &format!("{dir}/{name}"));
    let Some(manifest) = read("manifest.json")? else {
        if read("export.nt")?.is_some() {
            return Err(invalid(format!("{r}:{dir} missing manifest")));
        }
        return Ok(None);
    };
    let p = Pack {
        manifest: serde_json::from_str(&manifest).map_err(io)?,
        graph: read("export.nt")?.ok_or_else(|| invalid("missing export.nt"))?,
        shapes: read("shapes.ttl")?.ok_or_else(|| invalid("missing shapes.ttl"))?,
    };
    p.verify()?;
    Ok(Some(p))
}
fn same(a: &Pack, b: &Pack) -> bool {
    a.manifest == b.manifest && a.graph == b.graph && a.shapes == b.shapes
}
fn dirs(repo: &Path, refs: &[&str]) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    for r in refs {
        for p in text(repo, &["ls-tree", "-rz", "--name-only", r])?.split('\0') {
            if let Some(d) = p.strip_suffix("/export.nt") {
                out.insert(d.into());
            }
        }
    }
    Ok(out)
}
fn safe_dir(repo: &Path, dir: &str) -> Result<PathBuf> {
    let path = Path::new(dir);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid("qpack path must be repository-relative"));
    }
    let mut out = repo.to_path_buf();
    for part in path.components() {
        out.push(part);
        if out.is_symlink() {
            return Err(invalid("qpack path traverses a symlink"));
        }
    }
    for name in [
        "export.nt",
        "shapes.ttl",
        "manifest.json",
        "manifest.ttl",
        "export.ttl",
        "decisions.json",
    ] {
        if out.join(name).is_symlink() {
            return Err(invalid("qpack output is a symlink"));
        }
    }
    Ok(out)
}
fn refs(repo: &Path, base: &str, ours: &str, theirs: &str) -> Result<[String; 3]> {
    if !text(repo, &["rev-parse", "--show-prefix"])?
        .trim()
        .is_empty()
    {
        return Err(invalid("run qpack Git commands from the repository root"));
    }
    let r = [
        commit(repo, base)?,
        commit(repo, ours)?,
        commit(repo, theirs)?,
    ];
    let bases = text(repo, &["merge-base", "--all", &r[1], &r[2]])?;
    if bases.trim() != r[0] {
        return Err(invalid(
            "requires the unique Git merge-base (criss-cross merges refused)",
        ));
    }
    Ok(r)
}
fn from_env(repo: &Path) -> Result<[String; 3]> {
    let get = |k| {
        std::env::var(k).map_err(|_| {
            invalid("use quipu git-merge REF; standalone drivers require explicit snapshot context")
        })
    };
    refs(
        repo,
        &get("QUIPU_QPACK_BASE")?,
        &get("QUIPU_QPACK_OURS")?,
        &get("QUIPU_QPACK_THEIRS")?,
    )
}
fn build(repo: &Path, r: &[String; 3], dir: &str) -> Result<Merged> {
    let get = |rev: &str| {
        pack(repo, rev, dir)?
            .ok_or_else(|| invalid("driver requires an existing qpack in all three snapshots"))
    };
    plan(&get(&r[0])?, &get(&r[1])?, &get(&r[2])?)
}
/// Replace one file atomically: a temp file in the same directory, then rename.
fn replace(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(io)?;
    std::io::Write::write_all(&mut temp, bytes).map_err(io)?;
    temp.persist(dir.join(name)).map_err(io)?;
    Ok(())
}
fn write(dir: &Path, merged: &Merged) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(io)?;
    for (name, bytes) in [
        ("export.nt", merged.pack.graph.as_bytes().to_vec()),
        ("shapes.ttl", merged.pack.shapes.as_bytes().to_vec()),
        (
            "manifest.json",
            manifest_bytes(&merged.pack.manifest, true)?,
        ),
        (
            "manifest.ttl",
            manifest_turtle(&merged.pack.manifest).into_bytes(),
        ),
        (
            "decisions.json",
            serde_json::to_vec_pretty(&merged.decisions).map_err(io)?,
        ),
    ] {
        replace(dir, name, &bytes)?;
    }
    if dir.join("export.ttl").exists() {
        std::fs::remove_file(dir.join("export.ttl")).map_err(io)?;
    }
    Ok(())
}

/// A Git low-level driver. Returns false for unresolved decisions, with valid RDF in ours.
pub fn driver(repo: &Path, base: &Path, ours: &Path, theirs: &Path, path: &str) -> Result<bool> {
    let r = from_env(repo)?;
    let p = Path::new(path);
    let dir = p
        .parent()
        .and_then(Path::to_str)
        .ok_or_else(|| invalid("driver path has no qpack directory"))?;
    let name = p
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid("invalid payload name"))?;
    let output = safe_dir(repo, dir)?;
    for (rev, input) in r.iter().zip([base, ours, theirs]) {
        let expected =
            file(repo, rev, path)?.ok_or_else(|| invalid("missing driver input snapshot"))?;
        if std::fs::read(input).map_err(io)? != expected.as_bytes() {
            return Err(invalid("driver input does not match immutable snapshot"));
        }
    }
    let mut merged = build(repo, &r, dir)?;
    let ready = merged.apply(None)?;
    let bytes = match name {
        "export.nt" => merged.pack.graph.as_bytes().to_vec(),
        "shapes.ttl" => merged.pack.shapes.as_bytes().to_vec(),
        "manifest.json" => manifest_bytes(&merged.pack.manifest, true)?,
        "manifest.ttl" => manifest_turtle(&merged.pack.manifest).into_bytes(),
        _ => return Err(invalid("unsupported qpack driver file")),
    };
    std::fs::write(ours, bytes).map_err(io)?;
    if name == "export.nt" {
        std::fs::create_dir_all(&output).map_err(io)?;
        replace(
            &output,
            "decisions.json",
            &serde_json::to_vec_pretty(&merged.decisions).map_err(io)?,
        )?;
    }
    Ok(ready || name != "export.nt")
}

/// Start a local merge with immutable snapshot context, stopping before commit.
pub fn merge(repo: &Path, incoming: &str) -> Result<bool> {
    if !text(repo, &["status", "--porcelain"])?.trim().is_empty() {
        return Err(invalid("git-merge requires a clean worktree/index"));
    }
    let ours = commit(repo, "HEAD")?;
    let theirs = commit(repo, incoming)?;
    let base = text(repo, &["merge-base", "--all", &ours, &theirs])?;
    let r = refs(repo, base.trim(), &ours, &theirs)?;
    // Compute every overlapping qpack before Git mutates anything. Shapes conflicts
    // and incompatible envelopes fail here, irrespective of Git's per-file order.
    let mut plans = Vec::new();
    for dir in dirs(repo, &[&ours, &theirs])? {
        if let (Some(b), Some(o), Some(t)) = (
            pack(repo, &r[0], &dir)?,
            pack(repo, &ours, &dir)?,
            pack(repo, &theirs, &dir)?,
        ) && !same(&b, &o)
            && !same(&b, &t)
        {
            let mut merged = plan(&b, &o, &t)?;
            merged.apply(None)?;
            safe_dir(repo, &dir)?;
            plans.push((dir, merged));
        }
    }
    let exe = std::env::current_exe()
        .map_err(io)?
        .to_string_lossy()
        .replace('\'', "'\\''");
    let command = format!("'{exe}' merge-driver %O %A %B %P");
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            &format!("merge.quipu.driver={command}"),
            "merge",
            "--no-ff",
            "--no-commit",
            "--",
            &theirs,
        ])
        .env("QUIPU_QPACK_BASE", &r[0])
        .env("QUIPU_QPACK_OURS", &ours)
        .env("QUIPU_QPACK_THEIRS", &theirs)
        .status()
        .map_err(|e| spawn_git(&e))?;
    if !plans.is_empty() && !commit(repo, "MERGE_HEAD").is_ok_and(|head| head == theirs) {
        return Err(invalid(
            "Git did not enter the expected merge; no qpack outputs rewritten",
        ));
    }
    // Git may skip a driver when one file's bytes match. Reconcile whole qpacks
    // explicitly as well, and leave staging to the reviewer.
    let mut ready = status.success();
    for (dir, mut merged) in plans {
        ready &= merged.apply(None)?;
        write(&safe_dir(repo, &dir)?, &merged)?;
    }
    Ok(ready)
}

/// Recompute the plan from Git, validate a selected decision, and rehash the pack.
pub fn resolve(
    repo: &Path,
    base: &str,
    ours: &str,
    theirs: &str,
    dir: &str,
    key: &str,
    choice: &str,
) -> Result<bool> {
    let r = refs(repo, base, ours, theirs)?;
    let output = safe_dir(repo, dir)?;
    let mut d: Decisions =
        serde_json::from_slice(&std::fs::read(output.join("decisions.json")).map_err(io)?)
            .map_err(io)?;
    d.resolutions.insert(key.into(), choice.into());
    let mut merged = build(repo, &r, dir)?;
    let ready = merged.apply(Some(&d))?;
    write(&output, &merged)?;
    Ok(ready)
}

/// Mandatory CI verdict over immutable base/ours/theirs/result commits.
pub fn check(repo: &Path, base: &str, ours: &str, theirs: &str, result: &str) -> Result<usize> {
    let r = refs(repo, base, ours, theirs)?;
    let result = commit(repo, result)?;
    let mut count = 0;
    for dir in dirs(repo, &[&r[0], &r[1], &r[2], &result])? {
        let (b, o, t, actual) = (
            pack(repo, &r[0], &dir)?,
            pack(repo, &r[1], &dir)?,
            pack(repo, &r[2], &dir)?,
            pack(repo, &result, &dir)?,
        );
        let expected = match (b, o, t) {
            (Some(b), Some(o), Some(t)) if !same(&b, &o) && !same(&b, &t) => {
                let mut merged = plan(&b, &o, &t)?;
                let recorded = file(repo, &result, &format!("{dir}/decisions.json"))?
                    .map(|v| serde_json::from_str::<Decisions>(&v).map_err(io))
                    .transpose()?;
                if !merged.apply(recorded.as_ref())? {
                    return Err(invalid(format!(
                        "{dir}: unresolved conflicts/alias proposals"
                    )));
                }
                Some(merged.pack)
            }
            (Some(b), Some(o), Some(t)) => Some(if same(&b, &o) { t } else { o }),
            (None, Some(o), Some(t)) if same(&o, &t) => Some(o),
            (None, Some(o), None) | (None, None, Some(o)) => Some(o),
            (Some(b), Some(o), None) | (Some(b), None, Some(o)) if same(&b, &o) => None,
            (_, None, None) => None,
            _ => {
                return Err(invalid(format!(
                    "{dir}: incompatible add/add or delete/modify"
                )));
            }
        };
        match (expected, actual) {
            (None, None) => {}
            (Some(e), Some(a)) if same(&e, &a) => {
                validate(&a)?;
                if let Some(turtle) = file(repo, &result, &format!("{dir}/manifest.ttl"))?
                    && turtle != manifest_turtle(&a.manifest)
                {
                    return Err(invalid(format!("{dir}: stale derived manifest.ttl")));
                }
                count += 1;
            }
            _ => {
                return Err(invalid(format!(
                    "{dir}: committed qpack differs from shape-aware merge (including manifest/lineage)"
                )));
            }
        }
    }
    Ok(count)
}
