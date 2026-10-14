# cargo-rusticate

Start a production-ready, multi-tenant Rust backend from the [rusticate](https://github.com/ahadiihsan/rusticate) boilerplate:

```sh
cargo install cargo-rusticate
cargo rusticate new my-service --owner <github-user-or-org>
```

You get a workspace that already builds, tests and deploys: an HTTP API and a worker, platform crates (tenant isolation with PostgreSQL row-level security, idempotency, a transactional outbox to NATS JetStream, problem+json errors, OpenTelemetry), a reference bounded context, an engineering standard with machine-checked rules (`cargo xtask verify`), CI, a Dockerfile, Kubernetes manifests, and an operating manual for coding agents (`AGENTS.md`).

## Requirements

- To install: Rust 1.85 or newer, and `git`.
- To work on the generated project: rustup (the project pins its toolchain and rustup installs it) and Docker (local PostgreSQL and NATS).

## Usage

Create a project in a new directory:

```sh
cargo rusticate new my-service --owner acme
cd my-service
cargo xtask setup && cargo xtask db up && cargo xtask verify
```

Or fill a repository you already created and cloned (the directory may contain only `.git`, which is kept, remote included):

```sh
git clone git@github.com:acme/my-service.git && cd my-service
cargo rusticate init --owner acme
```

| Option | Meaning |
|---|---|
| `--owner <login>` | GitHub user or organisation that owns the repository. Written to `CODEOWNERS` and the `SECURITY.md` report link. Without it they say `OWNER`. |
| `--path <dir>` | (`new`) Directory to create. Default: `./<name>`. |
| `--name <name>` | (`init`) Project name. Default: the directory's name. |
| `--ref <tag-or-branch>` | Template version. Default: the release matching this generator (`v0.1.0` for 0.1.0). `--ref main` gives the latest unreleased template. |
| `--template <url-or-dir>` | Another source: a fork's git URL, or a local checkout of the template. |
| `--no-git` | Do not initialise a git repository. |

Project names are lowercase letters, digits and single hyphens, starting with a letter (`my-service`): the name is used for Docker, Kubernetes and image names, which allow nothing else.

## What the generator does

1. Fetches the template at the release matching its own version (`cargo-rusticate 0.1.0` → tag `v0.1.0`), so a given generator always produces the same project.
2. Renames the template's identity everywhere: `rusticate` → `my-service`, `Rusticate` → `My Service`, the repository URL and `@ahadiihsan` → your owner. Links into the template's history and the generator's own name are kept. Sections that only make sense in the template repository are removed.
3. Refuses to finish if any other spelling of the template's identity remains, and leaves the directory as it found it.
4. Starts a fresh git repository on `main` (no commit), unless the directory already had one or you passed `--no-git`.

The generated project keeps `LICENSE-MIT` and `LICENSE-APACHE`: they cover the code that came from the template. License your own code as you choose.

## Working on the template

The generator lives in the template repository, outside its Cargo workspace. Neither it nor the workflow that publishes it (`.github/workflows/release.yml`) is copied into generated projects.

- Everything added to the template ships to every new project. The rules in [`src/identity.rs`](src/identity.rs) decide what is renamed, kept or removed.
- `cargo xtask verify` formats, lints and tests the generator. `cargo xtask ci` also checks its dependencies, builds it with the declared minimum Rust version, generates a project from the working tree and runs that project's `verify --fast`.
- Releasing: [RELEASING.md](https://github.com/ahadiihsan/rusticate/blob/main/cargo-rusticate/RELEASING.md).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
