# cargo-rusticate

Start a production-ready, multi-tenant Rust backend from the [rusticate](https://github.com/ahadiihsan/rusticate) boilerplate:

```sh
cargo install cargo-rusticate
cargo rusticate new my-service --owner <github-user-or-org>
```

You get a workspace that already builds, tests and deploys: an HTTP API and a worker, platform crates (tenant isolation with PostgreSQL row-level security, idempotency, a transactional outbox to NATS JetStream, problem+json errors, OpenTelemetry), a reference bounded context, an engineering standard with machine-checked rules (`cargo rusticate verify`), CI, a Dockerfile, Kubernetes manifests, and an operating manual for coding agents (`AGENTS.md`).

## Requirements

- To install: Rust 1.85 or newer, and `git`.
- To work on the generated project: rustup (the project pins its toolchain and rustup installs it) and Docker (local PostgreSQL and NATS).

## Usage

Create a project in a new directory:

```sh
cargo rusticate new my-service --owner acme
cd my-service
cargo rusticate setup && cargo rusticate verify
```

In a terminal the generator asks which optional modules to include; each one builds on the one before it:

| Module | Adds | Needs |
|---|---|---|
| core (always) | HTTP API, auth, problem+json errors, telemetry, scaffolding, CI, agent harness | |
| `postgres` | PostgreSQL with row-level security (multi-tenant), migrations, idempotency | |
| `messaging` | NATS JetStream: worker, transactional outbox, event consumers | `postgres` |
| `example` | the `billing` context, a complete reference | `postgres`, `messaging` |

Without a terminal and without module flags, every module is included.

Or fill the current directory: an empty folder, or a repository you already created and cloned. It may contain only `.git` (kept, remote included) and OS files such as `.DS_Store`:

```sh
git clone git@github.com:acme/my-service.git && cd my-service
cargo rusticate init --owner acme
```

| Option | Meaning |
|---|---|
| `--owner <login>` | GitHub user or organisation that owns the repository. Written to `CODEOWNERS` and the `SECURITY.md` report link. Without it they say `OWNER`. |
| `--path <dir>` | (`new`) Directory to create. Default: `./<name>`. |
| `--name <name>` | (`init`) Project name. Default: the directory's name. |
| `--ref <tag-or-branch>` | Template version. Default: the release matching this generator (`v0.4.0` for 0.4.0). `--ref main` gives the latest unreleased template. |
| `--template <url-or-dir>` | Another source: a fork's git URL, or a local checkout of the template. |
| `--no-git` | Do not initialise a git repository. |
| `--minimal` | Core only; add modules with `--with`. |
| `--with <modules>` | Include these modules (comma-separated or repeated). |
| `--without <modules>` | Leave these out, and every module that needs them. |
| `--bare` | Everything but the example (same as `--without example`). |
| `--all` | Every module, without asking. |

Project names are lowercase letters, digits and single hyphens, starting with a letter (`my-service`): the name is used for Docker, Kubernetes and image names, which allow nothing else.

## Inside a project

Inside a project (any directory below one with `architecture.toml` and `xtask/`), every other command is the project's own `cargo xtask`:

```sh
cargo rusticate db up
cargo rusticate new domain shipping
cargo rusticate new migration shipping "create shipments"
cargo rusticate new feature shipping dispatch_parcel --kind command
cargo rusticate verify
cargo rusticate --help                  # lists every project command
```

`new` reads its first word: `domain`, `feature` or `migration` scaffolds inside the project; anything else creates a new project.

The commands are not built into this tool. They live in each project's `xtask/`, versioned with its code and CI, so a project keeps the rules it was generated with whatever version of `cargo-rusticate` is installed. `cargo xtask <command>` runs the same command without this tool (CI and the git hooks use it).

## Upgrading a project

```sh
cargo rusticate upgrade            # to this generator's template release
cargo rusticate upgrade --to main  # or any tag or branch
```

Merges the newer template's tooling and agent harness (xtask, scaffolds, skills, CI, docs, lint configs) into the project, keeping your changes: files you did not touch are replaced, files both sides changed are merged, and real conflicts are left as git conflict markers. Product code is never touched. It needs a clean working tree and the project's `rusticate.toml` (projects generated with 0.3.1 or later). See [ADR 0003](https://github.com/ahadiihsan/rusticate/blob/main/docs/adr/0003-upgrade-tooling-by-three-way-merge.md).

## What the generator does

1. Fetches the template at the release matching its own version (`cargo-rusticate 0.1.0` → tag `v0.1.0`), so a given generator always produces the same project.
2. Keeps the modules you chose: the template declares them in `template.toml`, with the paths each owns and the blocks it marks in shared files. A Rust file that lost lines is formatted with `rustfmt`, and a document link into a file that was left out points at that file in the template repository, at the version used.
3. Renames the template's identity everywhere: `rusticate` → `my-service`, `Rusticate` → `My Service`, the repository URL and `@ahadiihsan` → your owner. Links into the template's history and the generator's own name are kept. Sections that only make sense in the template repository are removed.
4. Refuses to finish if any other spelling of the template's identity remains, or a marker is malformed or names an unknown module, and leaves the directory as it found it.
5. Writes `rusticate.toml`: the template source, its exact revision, the modules chosen, the name and owner. Keep it committed; upgrading to a newer template compares against it.
6. Starts a fresh git repository on `main` (no commit), unless the directory already had one or you passed `--no-git`.

The generated project keeps `LICENSE-MIT` and `LICENSE-APACHE`: they cover the code that came from the template. License your own code as you choose.

## Working on the template

The generator lives in the template repository, outside its Cargo workspace. Neither it nor the workflow that publishes it (`.github/workflows/release.yml`) is copied into generated projects.

- Everything added to the template ships to every new project. The rules in [`src/identity.rs`](src/identity.rs) decide what is renamed, kept or removed.
- `cargo rusticate verify` formats, lints and tests the generator. `cargo rusticate ci` also checks its dependencies, builds it with the declared minimum Rust version, generates a project from the working tree and runs that project's `verify --fast`.
- Releasing: [RELEASING.md](https://github.com/ahadiihsan/rusticate/blob/main/cargo-rusticate/RELEASING.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
