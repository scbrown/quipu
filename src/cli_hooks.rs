//! `quipu hook <name>` (run one hook) and `quipu hooks bundle|install|
//! uninstall|status` (manage quipu's own hooks in Claude Code and Codex).
//! See [`crate::hooks_install`] for the install mechanism and
//! [`crate::hook_session_capture`] for the one hook quipu ships.

use crate::hooks_install::{self as hi, Harness};

const HOOKS_USAGE: &str = "usage: quipu hooks bundle
       quipu hooks install|uninstall|status [--harness claude|codex]... [--project] [--no-st]";

/// `quipu hook <name>`: run one hook. A hook ALWAYS exits 0 — a hook that
/// errors would surface as a harness failure on every stop — so an unknown
/// name prints a usage line on stderr and still exits 0.
pub fn cmd_hook(args: &[String]) {
    match args.get(2).map(String::as_str) {
        Some("session-capture") => crate::hook_session_capture::run_stdio(),
        _ => eprintln!("usage: quipu hook session-capture  (reads the Stop-hook JSON on stdin)"),
    }
}

/// Where to install, parsed from `--harness`, `--project` and `--no-st`.
struct Target {
    harnesses: Vec<Harness>,
    project: bool,
    no_st: bool,
}

fn parse_target(args: &[String]) -> Result<Target, String> {
    let mut t = Target {
        harnesses: Vec::new(),
        project: false,
        no_st: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--harness" => {
                let v = args.get(i + 1).ok_or("--harness needs a value")?;
                t.harnesses.push(Harness::parse(v)?);
                i += 1;
            }
            "--project" => t.project = true,
            "--no-st" => t.no_st = true,
            other => return Err(format!("unknown argument '{other}'")),
        }
        i += 1;
    }
    if t.harnesses.is_empty() {
        t.harnesses = vec![Harness::Claude, Harness::Codex];
    }
    Ok(t)
}

/// `quipu hooks …`. Exits 1 when status finds a harness incomplete, or on error.
pub fn cmd_hooks(args: &[String]) {
    let code = match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    };
    if code != 0 {
        std::process::exit(code);
    }
}

fn run(args: &[String]) -> Result<i32, String> {
    let action = args.get(2).map_or("", String::as_str);
    let bundle = hi::bundle();
    if action == "bundle" {
        println!(
            "{}",
            serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?
        );
        return Ok(0);
    }
    if !matches!(action, "install" | "uninstall" | "status") {
        return Err(HOOKS_USAGE.to_string());
    }
    let target = parse_target(&args[3..]).map_err(|e| format!("{e}\n{HOOKS_USAGE}"))?;
    if !target.no_st && !target.project && hi::st_available() {
        return via_st(action, &target);
    }
    let mut ok = true;
    for &h in &target.harnesses {
        let path = hi::config_path(h, target.project);
        let mut cfg = hi::read_config(&path, h)?;
        let name = h.name();
        match action {
            "install" => {
                let n = hi::merge(&mut cfg, &bundle, h);
                if n > 0 {
                    hi::write_config(&path, h, &cfg)?;
                }
                println!("{name}: added {n} hook(s) to {}", path.display());
            }
            "uninstall" => {
                let n = hi::remove(&mut cfg, &bundle, h);
                if n > 0 {
                    hi::write_config(&path, h, &cfg)?;
                }
                println!("{name}: removed {n} hook(s) from {}", path.display());
            }
            _ => {
                let (have, want) = hi::present(&cfg, &bundle, h);
                println!("{name}: {have}/{want} quipu hook(s) in {}", path.display());
                ok &= have == want;
            }
        }
    }
    Ok(i32::from(!ok))
}

/// With shantytown present, install/uninstall register the bundle and status
/// reads st's own check, so the answer reflects what st renders.
fn via_st(action: &str, target: &Target) -> Result<i32, String> {
    if action != "status" {
        let register = action == "install";
        println!("{}", hi::st_register(register)?);
        println!(
            "quipu hooks {} through shantytown: st renders them into every role's \
             claude and codex settings. Check with `st ops hooks check`.",
            if register {
                "registered"
            } else {
                "unregistered"
            }
        );
        return Ok(0);
    }
    let out = std::process::Command::new("st")
        .args(["ops", "hooks", "check", "--json"])
        .output()
        .map_err(|e| format!("cannot run st: {e}"))?;
    let report: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("st ops hooks check --json: {e}"))?;
    let mut ok = true;
    for &h in &target.harnesses {
        let name = h.name();
        let items: Vec<_> = report["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|i| i["bundle"] == "quipu" && i["harness"] == name)
            .collect();
        let count = |k: &str, v: &str| items.iter().filter(|i| i[k] == v).count();
        println!(
            "{name}: {} hook item(s) via st; configured ok {}, live ok {}, firing ok {}, silent {}",
            items.len(),
            count("configured", "ok"),
            count("live", "ok"),
            count("firing", "ok"),
            count("firing", "silent"),
        );
        ok &= !items.is_empty() && count("configured", "ok") == items.len();
    }
    Ok(i32::from(!ok))
}
