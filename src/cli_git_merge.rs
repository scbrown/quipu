//! Store-free Git transport commands; dispatched before loading store configuration.
use std::path::Path;

pub fn run(args: &[String]) {
    let repo = Path::new(".");
    let result = match (args[1].as_str(), &args[2..]) {
        ("merge-driver", [base, ours, theirs, path]) => quipu::git_merge::driver(
            repo,
            Path::new(base),
            Path::new(ours),
            Path::new(theirs),
            path,
        ),
        ("git-merge", [incoming]) => quipu::git_merge::merge(repo, incoming),
        ("qpack-resolve", [base, ours, theirs, dir, key, choice]) => {
            quipu::git_merge::resolve(repo, base, ours, theirs, dir, key, choice)
        }
        ("qpack-check", [base, ours, theirs, result]) => {
            quipu::git_merge::check(repo, base, ours, theirs, result).map(|n| {
                println!("qpack-check: {n} packs verified");
                true
            })
        }
        _ => {
            eprintln!(
                "usage: quipu git-merge REF | merge-driver BASE OURS THEIRS PATH | qpack-resolve BASE_REF OURS_REF THEIRS_REF DIR KEY CHOICE | qpack-check BASE_REF OURS_REF THEIRS_REF RESULT_REF"
            );
            std::process::exit(1);
        }
    };
    match result {
        Ok(true) => {}
        Ok(false) => {
            eprintln!(
                "qpack decisions pending: inspect decisions.json; resolve and git add before commit"
            );
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("qpack merge refused: {e}");
            std::process::exit(1);
        }
    }
}
