//! Process helpers. Every command is echoed before it runs, so humans and
//! agents can reproduce any step by hand.

use std::ffi::OsStr;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};

use crate::workspace;

pub fn cmd<I, S>(program: &str, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command.args(args).current_dir(workspace::root());
    command
}

pub fn cargo<I, S>(args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    cmd("cargo", args)
}

fn render(command: &Command) -> String {
    let args: Vec<_> = command
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    format!(
        "{} {}",
        command.get_program().to_string_lossy(),
        args.join(" ")
    )
}

/// Runs to completion with inherited stdio; fails on a non-zero exit.
pub fn run(mut command: Command) -> Result<()> {
    let rendered = render(&command);
    eprintln!("$ {rendered}");
    let status = command
        .status()
        .with_context(|| format!("failed to start `{rendered}`"))?;
    if !status.success() {
        bail!("`{rendered}` failed with {status}");
    }
    Ok(())
}

/// Runs and captures stdout; fails on a non-zero exit.
pub fn output(mut command: Command) -> Result<String> {
    let rendered = render(&command);
    eprintln!("$ {rendered}");
    let out = command
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("failed to start `{rendered}`"))?;
    if !out.status.success() {
        bail!("`{rendered}` failed with {}", out.status);
    }
    Ok(String::from_utf8(out.stdout)?)
}

/// Whether `tool` is on PATH (as itself, or as a cargo subcommand).
pub fn available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Runs a named step with a header, so CI logs read as a checklist.
pub fn step(name: &str, action: impl FnOnce() -> Result<()>) -> Result<()> {
    eprintln!("\n━━ {name} ━━");
    action().with_context(|| format!("step `{name}` failed"))
}
