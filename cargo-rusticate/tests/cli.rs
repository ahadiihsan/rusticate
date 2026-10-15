//! Black-box tests: the real binary against a small fake template.
#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A temporary directory removed at the end of the test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "cargo-rusticate-test-{}-{n}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write(root: &Path, file: &str, content: &str) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn read(root: &Path, file: &str) -> String {
    fs::read_to_string(root.join(file)).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

const BINARY: &[u8] = &[
    0xff, 0xfe, b'r', b'u', b's', b't', b'i', b'c', b'a', b't', b'e',
];

/// A minimal template with the identity in every form the generator handles.
fn fake_template(root: &Path) {
    write(root, "AGENTS.md", "# rusticate agents\n");
    write(root, "architecture.toml", "# Rusticate architecture\n");
    write(root, ".github/CODEOWNERS", "*  @ahadiihsan\n");
    write(
        root,
        "SECURITY.md",
        "[report](https://github.com/ahadiihsan/rusticate/security/advisories/new)\n",
    );
    write(
        root,
        "docs/history.md",
        "[review](https://github.com/ahadiihsan/rusticate/blob/af3c/review.md)\n",
    );
    write(
        root,
        "README.md",
        "# rusticate\n\n<!-- rusticate:template-only:start -->\n## Start a new project\n\n\
         cargo install cargo-rusticate\n\n<!-- rusticate:template-only:end -->\n## Quickstart\n",
    );
    write(
        root,
        "cargo-rusticate/Cargo.toml",
        "[package]\nname = \"cargo-rusticate\"\n",
    );
    write(root, ".github/workflows/ci.yml", "name: ci\n");
    write(
        root,
        ".github/workflows/release.yml",
        "run: cargo publish -p cargo-rusticate\n",
    );
    write(root, ".gitignore", "/target\n");
    write(root, "target/debug/stale", "rusticate\n");
    fs::write(root.join("logo.bin"), BINARY).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        write(root, "scripts/hook.sh", "#!/bin/sh\necho rusticate\n");
        fs::set_permissions(
            root.join("scripts/hook.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        write(root, ".agents/skills/x/SKILL.md", "skill\n");
        fs::create_dir_all(root.join(".claude")).unwrap();
        std::os::unix::fs::symlink("../.agents/skills", root.join(".claude/skills")).unwrap();
    }
    git(root, &["init", "--quiet"]);
}

/// The fake template as a git remote with an annotated release tag, like the
/// real one. Returns its URL.
fn released_template(root: &Path) -> String {
    fake_template(root);
    let identity = [
        "-c",
        "user.name=Template",
        "-c",
        "user.email=template@example.com",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "tag.gpgsign=false",
    ];
    git(root, &["add", "-A"]);
    git(
        root,
        &[&identity[..], &["commit", "--quiet", "-m", "template"]].concat(),
    );
    git(
        root,
        &[&identity[..], &["tag", "-a", "v9.9.9", "-m", "release"]].concat(),
    );
    format!("file://{}", root.display())
}

fn rusticate(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-rusticate"))
        .current_dir(cwd)
        .arg("rusticate")
        .args(args)
        .output()
        .unwrap()
}

fn assert_ok(out: &Output) {
    assert!(
        out.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn new_creates_a_project_with_the_new_identity() {
    let template = Scratch::new("template");
    fake_template(template.path());
    let work = Scratch::new("work");
    let tpl = template.path().to_str().unwrap();

    let out = rusticate(
        work.path(),
        &["new", "acme-ledger", "--owner", "acme", "--template", tpl],
    );

    assert_ok(&out);
    let project = work.path().join("acme-ledger");
    assert_eq!(read(&project, "AGENTS.md"), "# acme-ledger agents\n");
    assert_eq!(
        read(&project, "architecture.toml"),
        "# Acme Ledger architecture\n"
    );
    assert_eq!(read(&project, ".github/CODEOWNERS"), "*  @acme\n");
    assert_eq!(
        read(&project, "SECURITY.md"),
        "[report](https://github.com/acme/acme-ledger/security/advisories/new)\n"
    );
    assert_eq!(
        read(&project, "docs/history.md"),
        "[review](https://github.com/ahadiihsan/rusticate/blob/af3c/review.md)\n",
        "history links keep pointing at the template"
    );
    assert_eq!(
        read(&project, "README.md"),
        "# acme-ledger\n\n## Quickstart\n"
    );
    assert!(
        !project.join("cargo-rusticate").exists(),
        "the generator is not copied"
    );
    assert!(
        !project.join(".github/workflows/release.yml").exists(),
        "nor is the workflow that publishes it"
    );
    assert!(project.join(".github/workflows/ci.yml").exists());
    assert!(
        !project.join("target").exists(),
        "ignored files are not copied"
    );
    assert_eq!(
        fs::read(project.join("logo.bin")).unwrap(),
        BINARY,
        "binary files are untouched"
    );
    assert_eq!(
        git(&project, &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/main"
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("cd "));
}

#[cfg(unix)]
#[test]
fn new_keeps_symlinks_and_executable_bits() {
    use std::os::unix::fs::PermissionsExt;
    let template = Scratch::new("template");
    fake_template(template.path());
    let work = Scratch::new("work");

    let out = rusticate(
        work.path(),
        &[
            "new",
            "svc",
            "--owner",
            "acme",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );

    assert_ok(&out);
    let project = work.path().join("svc");
    let link = fs::read_link(project.join(".claude/skills")).unwrap();
    assert_eq!(link, Path::new("../.agents/skills"));
    assert_eq!(read(&project, ".claude/skills/x/SKILL.md"), "skill\n");
    let mode = fs::metadata(project.join("scripts/hook.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "scripts stay executable");
    assert_eq!(read(&project, "scripts/hook.sh"), "#!/bin/sh\necho svc\n");
}

#[test]
fn init_fills_a_cloned_empty_repository_and_keeps_its_git() {
    let template = Scratch::new("template");
    fake_template(template.path());
    let work = Scratch::new("work");
    let repo = work.path().join("payments-api");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet"]);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:acme/payments-api.git",
        ],
    );

    let out = rusticate(
        &repo,
        &[
            "init",
            "--owner",
            "acme",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );

    assert_ok(&out);
    assert_eq!(
        read(&repo, "AGENTS.md"),
        "# payments-api agents\n",
        "name from the directory"
    );
    assert_eq!(
        git(&repo, &["remote", "get-url", "origin"]).trim(),
        "git@github.com:acme/payments-api.git"
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("cd "),
        "already in the project"
    );
}

#[test]
fn init_refuses_a_directory_with_files_and_leaves_it_untouched() {
    let template = Scratch::new("template");
    fake_template(template.path());
    let dir = Scratch::new("occupied");
    write(dir.path(), "notes.txt", "mine\n");

    let out = rusticate(
        dir.path(),
        &[
            "init",
            "--name",
            "svc",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );

    assert!(!out.status.success());
    assert!(stderr(&out).contains("is not empty"), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("`notes.txt`"),
        "names what is in the way: {}",
        stderr(&out)
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(read(dir.path(), "notes.txt"), "mine\n");
}

#[test]
fn new_refuses_a_non_empty_destination() {
    let template = Scratch::new("template");
    fake_template(template.path());
    let work = Scratch::new("work");
    write(work.path(), "svc/keep.txt", "keep\n");

    let out = rusticate(
        work.path(),
        &[
            "new",
            "svc",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );

    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("is not empty (it contains `keep.txt`)"),
        "{}",
        stderr(&out)
    );
    assert_eq!(read(work.path(), "svc/keep.txt"), "keep\n");
}

#[test]
fn invalid_names_and_owners_are_rejected_before_anything_is_written() {
    let work = Scratch::new("work");
    let bad_name = rusticate(work.path(), &["new", "My_Service"]);
    assert!(
        stderr(&bad_name).contains("invalid project name"),
        "{}",
        stderr(&bad_name)
    );
    let bad_owner = rusticate(work.path(), &["new", "svc", "--owner", "acme/team"]);
    assert!(
        stderr(&bad_owner).contains("invalid GitHub owner"),
        "{}",
        stderr(&bad_owner)
    );
    assert_eq!(fs::read_dir(work.path()).unwrap().count(), 0);
}

#[test]
fn uncovered_identity_aborts_and_removes_the_new_directory() {
    let template = Scratch::new("template");
    fake_template(template.path());
    write(template.path(), "docs/shout.md", "WELCOME TO RUSTICATE\n");
    let work = Scratch::new("work");

    let out = rusticate(
        work.path(),
        &[
            "new",
            "svc",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );

    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("docs/shout.md: line 1") && err.contains("`RUSTICATE`"),
        "{err}"
    );
    assert!(
        !work.path().join("svc").exists(),
        "a failed run leaves nothing behind"
    );
}

#[test]
fn a_directory_that_is_not_a_template_is_refused() {
    let not_template = Scratch::new("not-template");
    write(not_template.path(), "README.md", "hello\n");
    git(not_template.path(), &["init", "--quiet"]);
    let work = Scratch::new("work");

    let out = rusticate(
        work.path(),
        &[
            "new",
            "svc",
            "--template",
            not_template.path().to_str().unwrap(),
        ],
    );

    assert!(
        stderr(&out).contains("is not a rusticate template"),
        "{}",
        stderr(&out)
    );
    assert!(!work.path().join("svc").exists());
}

#[test]
fn missing_owner_warns_and_leaves_a_placeholder() {
    let template = Scratch::new("template");
    fake_template(template.path());
    let work = Scratch::new("work");

    let out = rusticate(
        work.path(),
        &[
            "new",
            "svc",
            "--no-git",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );

    assert_ok(&out);
    assert!(
        stderr(&out).contains("no --owner given"),
        "{}",
        stderr(&out)
    );
    let project = work.path().join("svc");
    assert_eq!(read(&project, ".github/CODEOWNERS"), "*  @OWNER\n");
    assert!(!project.join(".git").exists(), "--no-git skips git init");
}

#[test]
fn runs_without_the_cargo_prefix_and_reports_its_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_cargo-rusticate"))
        .arg("--version")
        .output()
        .unwrap();
    assert_ok(&out);
    assert!(String::from_utf8_lossy(&out.stdout).contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_release_tag_is_fetched_from_git_without_any_output_from_git() {
    let template = Scratch::new("remote");
    let url = released_template(template.path());
    let work = Scratch::new("work");

    let out = rusticate(
        work.path(),
        &[
            "new",
            "svc",
            "--owner",
            "acme",
            "--template",
            &url,
            "--ref",
            "v9.9.9",
        ],
    );

    assert_ok(&out);
    assert_eq!(stderr(&out), "", "git must not talk on success");
    let project = work.path().join("svc");
    assert_eq!(read(&project, "AGENTS.md"), "# svc agents\n");
    assert!(!project.join("cargo-rusticate").exists());
    assert_eq!(
        git(&project, &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/main"
    );
    assert_eq!(
        git(&project, &["rev-list", "--all", "--count"]).trim(),
        "0",
        "no template history"
    );
}

#[test]
fn without_a_ref_a_git_template_is_taken_from_its_default_branch() {
    let template = Scratch::new("remote");
    let url = released_template(template.path());
    let work = Scratch::new("work");

    let out = rusticate(
        work.path(),
        &["new", "svc", "--owner", "acme", "--template", &url],
    );

    assert_ok(&out);
    assert_eq!(
        read(&work.path().join("svc"), "AGENTS.md"),
        "# svc agents\n"
    );
}

#[test]
fn a_missing_release_is_a_clear_error_and_leaves_nothing() {
    let template = Scratch::new("remote");
    let url = released_template(template.path());
    let work = Scratch::new("work");

    let out = rusticate(
        work.path(),
        &["new", "svc", "--template", &url, "--ref", "v0.0.0"],
    );

    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("could not fetch the template") && err.contains("`v0.0.0`"),
        "{err}"
    );
    assert!(
        err.contains("couldn't find remote ref"),
        "git's own reason is shown: {err}"
    );
    assert!(!work.path().join("svc").exists());
}

/// A minimal project: the two markers and a real `xtask` that echoes its
/// arguments, behind the same `cargo xtask` alias the template defines.
fn fake_project(root: &Path) {
    write(root, "architecture.toml", "");
    write(
        root,
        "Cargo.toml",
        "[workspace]\nresolver = \"3\"\nmembers = [\"xtask\"]\n",
    );
    write(
        root,
        ".cargo/config.toml",
        "[alias]\nxtask = \"run --quiet --package xtask --\"\n",
    );
    write(
        root,
        "xtask/Cargo.toml",
        "[package]\nname = \"xtask\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    );
    write(
        root,
        "xtask/src/main.rs",
        "fn main() {\n    let args: Vec<String> = std::env::args().skip(1).collect();\n    \
         println!(\"xtask {}\", args.join(\" \"));\n    \
         if args.first().is_some_and(|a| a == \"fail\") {\n        std::process::exit(7);\n    }\n}\n",
    );
    fs::create_dir_all(root.join("crates/billing/src")).unwrap();
}

fn rusticate_in_project(project: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-rusticate"))
        .current_dir(cwd)
        .env("CARGO_TARGET_DIR", project.join("target"))
        .arg("rusticate")
        .args(args)
        .output()
        .unwrap()
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn inside_a_project_every_other_command_is_its_xtask() {
    let dir = Scratch::new("project");
    let project = dir.path();
    fake_project(project);

    let db = rusticate_in_project(project, project, &["db", "up"]);
    assert_ok(&db);
    assert_eq!(stdout(&db), "xtask db up\n");

    let scaffold = rusticate_in_project(project, project, &["new", "domain", "shipping"]);
    assert_ok(&scaffold);
    assert_eq!(stdout(&scaffold), "xtask new domain shipping\n");

    let nested = project.join("crates/billing/src");
    let from_below = rusticate_in_project(project, &nested, &["verify", "--fast"]);
    assert_ok(&from_below);
    assert_eq!(stdout(&from_below), "xtask verify --fast\n");

    let failing = rusticate_in_project(project, project, &["fail"]);
    assert_eq!(
        failing.status.code(),
        Some(7),
        "the xtask exit code is kept"
    );
}

#[test]
fn outside_a_project_a_project_command_is_explained() {
    let dir = Scratch::new("nowhere");

    let out = rusticate(dir.path(), &["db", "up"]);

    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("`db` is a project command"), "{err}");
    assert!(err.contains("cargo rusticate new <name>"), "{err}");
}

#[test]
fn init_ignores_os_clutter_and_keeps_it() {
    let template = Scratch::new("template");
    fake_template(template.path());
    let dir = Scratch::new("finder-opened");
    write(dir.path(), ".DS_Store", "finder\n");

    let out = rusticate(
        dir.path(),
        &[
            "init",
            "--name",
            "svc",
            "--owner",
            "acme",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );

    assert_ok(&out);
    assert_eq!(read(dir.path(), "AGENTS.md"), "# svc agents\n");
    assert_eq!(read(dir.path(), ".DS_Store"), "finder\n");
}

#[test]
fn new_ignores_os_clutter_in_an_existing_destination() {
    let template = Scratch::new("template");
    fake_template(template.path());
    let work = Scratch::new("work");
    write(work.path(), "svc/Thumbs.db", "explorer\n");

    let out = rusticate(
        work.path(),
        &[
            "new",
            "svc",
            "--owner",
            "acme",
            "--template",
            template.path().to_str().unwrap(),
        ],
    );

    assert_ok(&out);
    assert_eq!(
        read(&work.path().join("svc"), "AGENTS.md"),
        "# svc agents\n"
    );
}

/// A fake template with two modules: `db` (owns `db/`, a block in
/// `src/main.rs`, and a stand-in `nodb.txt`) and `queue` (needs `db`).
fn modular_template(root: &Path) {
    fake_template(root);
    write(
        root,
        "template.toml",
        "template_only = [\"tools/release.sh\"]\n\n\
         [[module]]\nname = \"db\"\nsummary = \"a database\"\npaths = [\"db\"]\nabsent_paths = [\"nodb.txt\"]\n\n\
         [[module]]\nname = \"queue\"\nsummary = \"a queue\"\nrequires = [\"db\"]\npaths = [\"queue.txt\"]\n",
    );
    write(root, "db/schema.sql", "-- rusticate tables\n");
    write(root, "nodb.txt", "no database\n");
    write(root, "queue.txt", "queue\n");
    write(root, "tools/release.sh", "release rusticate\n");
    write(
        root,
        "src/main.rs",
        "fn main() {\n    // rusticate:module(db):start\n    db();\n    // rusticate:module(db):end\n    \
         // rusticate:module(queue):start\n    queue();\n    // rusticate:module(queue):end\n}\n",
    );
    git(root, &["add", "-A"]);
}

fn generate(template: &Path, work: &Path, extra: &[&str]) -> Output {
    let tpl = template.to_str().unwrap();
    let mut args = vec![
        "new",
        "svc",
        "--owner",
        "acme",
        "--no-git",
        "--template",
        tpl,
    ];
    args.extend_from_slice(extra);
    rusticate(work, &args)
}

#[test]
fn without_flags_every_module_is_included_and_markers_are_removed() {
    let template = Scratch::new("template");
    modular_template(template.path());
    let work = Scratch::new("work");

    let out = generate(template.path(), work.path(), &[]);

    assert_ok(&out);
    let project = work.path().join("svc");
    assert_eq!(
        read(&project, "src/main.rs"),
        "fn main() {\n    db();\n    queue();\n}\n"
    );
    assert_eq!(read(&project, "db/schema.sql"), "-- svc tables\n");
    assert!(project.join("queue.txt").exists());
    assert!(
        !project.join("nodb.txt").exists(),
        "stand-ins go when their module is in"
    );
    assert!(
        !project.join("tools/release.sh").exists(),
        "template-only paths never ship"
    );
    assert!(!project.join("template.toml").exists());
    let record = read(&project, "rusticate.toml");
    assert!(record.contains("modules = [\"db\", \"queue\"]"), "{record}");
    assert!(
        record.contains("name = \"svc\"") && record.contains("owner = \"acme\""),
        "{record}"
    );
    assert!(
        stdout(&out).contains("core + db + queue"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn minimal_keeps_only_core_and_the_stand_ins() {
    let template = Scratch::new("template");
    modular_template(template.path());
    let work = Scratch::new("work");

    let out = generate(template.path(), work.path(), &["--minimal"]);

    assert_ok(&out);
    let project = work.path().join("svc");
    assert_eq!(read(&project, "src/main.rs"), "fn main() {\n}\n");
    assert!(!project.join("db").exists());
    assert!(!project.join("queue.txt").exists());
    assert_eq!(read(&project, "nodb.txt"), "no database\n");
    assert!(stdout(&out).contains("core only"), "{}", stdout(&out));
}

#[test]
fn without_drops_dependents_and_unmet_requirements_are_refused() {
    let template = Scratch::new("template");
    modular_template(template.path());
    let work = Scratch::new("work");

    let out = generate(template.path(), work.path(), &["--without", "db"]);
    assert_ok(&out);
    assert!(
        !work.path().join("svc/queue.txt").exists(),
        "queue needs db"
    );

    let elsewhere = Scratch::new("elsewhere");
    let refused = generate(
        template.path(),
        elsewhere.path(),
        &["--minimal", "--with", "queue"],
    );
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("`queue` needs `db`"),
        "{}",
        stderr(&refused)
    );
}

#[test]
fn a_marker_naming_an_unknown_module_aborts_generation() {
    let template = Scratch::new("template");
    modular_template(template.path());
    write(
        template.path(),
        "src/lib.rs",
        "// rusticate:module(dbb):start\nfn typo() {}\n// rusticate:module(dbb):end\n",
    );
    git(template.path(), &["add", "-A"]);
    let work = Scratch::new("work");

    let out = generate(template.path(), work.path(), &[]);

    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("src/lib.rs: line 1") && err.contains("unknown module `dbb`"),
        "{err}"
    );
    assert!(!work.path().join("svc").exists());
}

const AUTHOR: [&str; 8] = [
    "-c",
    "user.name=T",
    "-c",
    "user.email=t@example.com",
    "-c",
    "commit.gpgsign=false",
    "-c",
    "tag.gpgsign=false",
];

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[&AUTHOR[..], &["commit", "--quiet", "-m", message]].concat(),
    );
}

#[test]
fn upgrade_merges_template_changes_with_local_ones() {
    let template = Scratch::new("upgrade-template");
    fake_template(template.path());
    write(
        template.path(),
        "xtask/Cargo.toml",
        "[package]\nname = \"xtask\"\n",
    );
    write(template.path(), "xtask/notes.md", "one\ntwo\nthree\n");
    write(template.path(), "src/product.rs", "product v1\n");
    commit_all(template.path(), "v1");
    git(
        template.path(),
        &[&AUTHOR[..], &["tag", "-a", "v1", "-m", "v1"]].concat(),
    );
    let url = format!("file://{}", template.path().display());

    let work = Scratch::new("upgrade-work");
    let generated = rusticate(
        work.path(),
        &[
            "new",
            "svc",
            "--owner",
            "acme",
            "--template",
            &url,
            "--ref",
            "v1",
        ],
    );
    assert_ok(&generated);
    let project = work.path().join("svc");
    commit_all(&project, "generated");
    write(&project, "xtask/notes.md", "ONE (mine)\ntwo\nthree\n");
    commit_all(&project, "local change");

    write(
        template.path(),
        "xtask/notes.md",
        "one\ntwo\nTHREE (template)\n",
    );
    write(template.path(), "xtask/added.md", "new tool\n");
    write(template.path(), "src/product.rs", "product v2\n");
    commit_all(template.path(), "v2");
    git(
        template.path(),
        &[&AUTHOR[..], &["tag", "-a", "v2", "-m", "v2"]].concat(),
    );

    let out = rusticate(&project, &["upgrade", "--to", "v2"]);

    assert_ok(&out);
    assert_eq!(
        read(&project, "xtask/notes.md"),
        "ONE (mine)\ntwo\nTHREE (template)\n"
    );
    assert_eq!(read(&project, "xtask/added.md"), "new tool\n");
    assert_eq!(
        read(&project, "src/product.rs"),
        "product v1\n",
        "product code is untouched"
    );
    assert!(read(&project, "rusticate.toml").contains("revision = \"v2\""));
    assert!(
        stdout(&out).contains("1 added") && stdout(&out).contains("1 merged"),
        "{}",
        stdout(&out)
    );
}

#[test]
fn upgrade_refuses_a_dirty_working_tree() {
    let template = Scratch::new("dirty-template");
    fake_template(template.path());
    write(
        template.path(),
        "xtask/Cargo.toml",
        "[package]\nname = \"xtask\"\n",
    );
    commit_all(template.path(), "v1");
    git(
        template.path(),
        &[&AUTHOR[..], &["tag", "-a", "v1", "-m", "v1"]].concat(),
    );
    let url = format!("file://{}", template.path().display());
    let work = Scratch::new("dirty-work");
    assert_ok(&rusticate(
        work.path(),
        &["new", "svc", "--template", &url, "--ref", "v1"],
    ));
    let project = work.path().join("svc");

    let out = rusticate(&project, &["upgrade", "--to", "v1"]);

    assert!(!out.status.success());
    assert!(stderr(&out).contains("commit or stash"), "{}", stderr(&out));
}
