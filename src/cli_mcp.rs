//! The local MCP command shares the companion server's startup and policy.
pub(crate) fn run(args: &[String]) -> ! {
    let executable = std::env::current_exe().unwrap_or_else(|error| {
        eprintln!("cannot locate Quipu installation: {error}");
        std::process::exit(1);
    });
    let server = executable.with_file_name(if cfg!(windows) {
        "quipu-server.exe"
    } else {
        "quipu-server"
    });
    // No PATH search: a release installs this reviewed sibling beside the CLI.
    // The child inherits the transport streams, not a token environment export.
    let mut command = std::process::Command::new(server);
    command.arg("--mcp-stdio").args(args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        eprintln!("cannot exec companion quipu-server (install it beside quipu): {error}");
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    match command.status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("cannot start companion quipu-server (install it beside quipu): {error}");
            std::process::exit(1);
        }
    }
}
