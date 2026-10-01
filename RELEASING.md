# Releasing lnurlcash-core

Crates.io versions are permanent. The release workflow validates the exact tag
before the protected publish job can use a registry token.

## One-time setup

The publish job uses crates.io trusted publishing: GitHub vouches for the
workflow over OIDC and crates.io hands back a token that lives for one run. No
registry secret is stored anywhere.

1. On crates.io, open the `lnurlcash-core` crate's Settings, Trusted
   Publishing, and add a GitHub publisher: owner `lnurlcash`, repository
   `lnurlcash-core`, workflow `release.yml`, environment `crates-io`.
2. In this repository, keep the `crates-io` GitHub environment restricted to
   `v*.*.*` tags, with required reviewers if every publish should be approved.
3. Add the other maintainers or an appropriate GitHub team as crate owners. Do
   not put a personal registry token in repository secrets or a local release
   script. (A `CARGO_REGISTRY_TOKEN` environment secret still works, and takes
   precedence, but should only bridge until trusted publishing is set up.)

## Rehearsal

Run the `release` workflow manually from `main` with the intended tag. The tag
is prospective and must not exist yet. This runs the full conformance suite and
`cargo publish --locked --dry-run` without entering the publishing environment
or reading its secret.

## Release

1. Date the matching changelog entry. The version is not in the repository:
   `Cargo.toml` carries a `0.0.0` placeholder, and the release workflow stamps
   the tag's version into `Cargo.toml` and `Cargo.lock` before it tests and
   publishes.
2. Merge only after CI and the local package dry-run pass.
3. Create and push the exact version tag, for example `v0.1.0`.
4. The tag runs the same validation and then enters the protected `crates-io`
   environment. Once approved, it runs `cargo publish --locked`.
5. Verify the version and repository link on crates.io before creating the
   matching GitHub release.

Never reuse or move a published version tag.
