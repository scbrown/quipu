//! Exclusive private claim, retained even when a capture attempt fails.
#[cfg(unix)]
pub(super) fn claim(db_path: &str, nonce: &str) -> std::io::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let invalid = || std::io::Error::other("private claim refused");
    let db = std::fs::canonicalize(db_path)?;
    let parent = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(db.parent().ok_or_else(invalid)?)?;
    match rustix::fs::mkdirat(
        &parent,
        "query-captures",
        Mode::RUSR | Mode::WUSR | Mode::XUSR,
    ) {
        Ok(()) => (),
        Err(rustix::io::Errno::EXIST) => (),
        Err(error) => return Err(error.into()),
    }
    let dir: std::fs::File = rustix::fs::openat(
        &parent,
        "query-captures",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into();
    let metadata = dir.metadata()?;
    if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o077 != 0 {
        return Err(invalid());
    }
    let file = rustix::fs::openat(
        &dir,
        format!("{nonce}.json"),
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )?;
    let file: std::fs::File = file.into();
    // Make the consumed nonce durable before any request becomes eligible.
    file.sync_all()?;
    dir.sync_all()?;
    parent.sync_all()?;
    Ok(file)
}

#[cfg(not(unix))]
pub(super) fn claim(_: &str, _: &str) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::other("private capture unsupported"))
}
