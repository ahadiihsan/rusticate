# Releasing rusticate

The template and the generator share one version. `cargo-rusticate X.Y.Z` creates projects from the template at tag `vX.Y.Z`, so the tag must exist when the crate is published, and must never move afterwards.

## Steps

1. **Bump.** In a pull request, set `version` in [`Cargo.toml`](Cargo.toml) and refresh the lock:

   ```sh
   cargo update --manifest-path cargo-rusticate/Cargo.toml --package cargo-rusticate
   ```

   Merge it once CI is green. CI's smoke test proves the template at this commit generates a project that passes `verify --fast`.

2. **Tag** the merge commit on `main` and push the tag:

   ```sh
   git switch main && git pull --ff-only
   git tag -a vX.Y.Z -m "rusticate vX.Y.Z"
   git push origin vX.Y.Z
   ```

   The [`release` workflow](../.github/workflows/release.yml) then checks that the tag matches the generator's version and is on `main`, runs the generator's tests and publishes it to crates.io.

3. **Check** what users get:

   ```sh
   cargo install cargo-rusticate --locked --version X.Y.Z
   cargo rusticate new probe --owner acme --path /tmp/probe
   cd /tmp/probe && cargo rusticate verify --fast
   ```

A broken release is fixed forward: release `X.Y.Z+1`, then `cargo yank --version X.Y.Z cargo-rusticate`. Never move or delete a release tag; installed generators keep fetching it.

If the workflow cannot run, publish from a clean checkout of the tag with `cargo publish --manifest-path cargo-rusticate/Cargo.toml --locked` (needs `cargo login`).

## The crates.io token

The workflow publishes with crates.io [trusted publishing](https://crates.io/docs/trusted-publishing): GitHub issues it a short-lived token per run, so no long-lived token is stored. It is configured once on crates.io (crate settings → Trusted Publishing: repository `ahadiihsan/rusticate`, workflow `release.yml`). The rust-lang auth action must be allowed in the repository's Actions settings (see `docs/operations.md`).

## Release tags are protected

A repository ruleset blocks updating or deleting `v*` tags. To recreate it:

```sh
gh api -X POST repos/OWNER/REPO/rulesets --input - <<'JSON'
{
  "name": "release tags",
  "target": "tag",
  "enforcement": "active",
  "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } },
  "rules": [{ "type": "update" }, { "type": "deletion" }, { "type": "non_fast_forward" }]
}
JSON
```
