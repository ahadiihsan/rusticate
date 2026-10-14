//! Running git. Its output stays out of the user's terminal unless a command
//! fails; then it becomes part of the error.

use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context as _, Result, bail};

/// Runs `git <args>` in `dir` and returns its stdout.
pub(crate) fn output(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let Output {
        status,
        stdout,
        stderr,
    } = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .context("could not run `git`: is it installed?")?;
    if !status.success() {
        bail!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    Ok(stdout)
}

pub(crate) fn run(dir: &Path, args: &[&str]) -> Result<()> {
    output(dir, args).map(drop)
}

/// A new repository on `main`. The `-c` keeps newer git from printing its
/// default-branch hint; `symbolic-ref` covers git older than 2.28.
pub(crate) fn init(dir: &Path) -> Result<()> {
    run(dir, &["-c", "init.defaultBranch=main", "init", "--quiet"])?;
    run(dir, &["symbolic-ref", "HEAD", "refs/heads/main"])
}

/// Checks out `git_ref` (a tag or branch; the default branch if `None`) of the
/// repository at `url` into the empty directory `into`, without history.
///
/// Fetching the one ref avoids `git clone --depth 1 --branch <tag>`, which
/// warns "... is not a commit!" for every annotated tag (all releases).
pub(crate) fn shallow_checkout(url: &str, git_ref: Option<&str>, into: &Path) -> Result<()> {
    init(into)?;
    let wanted = git_ref.unwrap_or("HEAD");
    run(
        into,
        &["fetch", "--quiet", "--depth", "1", "--", url, wanted],
    )?;
    run(
        into,
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "--quiet",
            "FETCH_HEAD",
        ],
    )
}
