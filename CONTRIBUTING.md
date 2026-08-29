# Contributing to waxum

Thanks for your interest in improving waxum. This document covers the
development workflow, quality gates every push must pass, and the
conventions the project follows.

## Where to start

If you're looking for a first contribution, start here:

**[Open `good first issue` tickets](https://github.com/imtaqin/waxum/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)**

Most of what's there right now is handler test coverage. Roughly half of
waxum's HTTP surface has no integration test, and each untested handler
module is a self-contained, low-context piece of work that mirrors a test
file already in the repo. Each issue names the handler file, lists the
exact routes to cover, and says which existing test file to mirror, so
you shouldn't need to go spelunking to get started.

[`tests/presence.rs`](tests/presence.rs) is the worked example — read its
header first. It spells out the four assertions that apply to every
session-scoped handler, and where the boundary sits: assert the HTTP
contract, not the protocol behaviour. Anything past the `get_client` gate
needs a live WhatsApp client and is out of scope. For a module with more
than a few routes, mirror the table-sweep shape in
[`tests/groups_management.rs`](tests/groups_management.rs),
[`tests/newsletter.rs`](tests/newsletter.rs), or
[`tests/labels.rs`](tests/labels.rs) instead.

Finding a real defect while writing a test is a good outcome — file it
separately rather than fixing it in the test PR. That has already
happened once here (#90 surfaced the bug fixed by #93).

## Getting the code

```sh
git clone https://github.com/imtaqin/waxum.git
cd waxum
```

The crate targets Rust **nightly** because the upstream `whatsapp-rust`
client relies on the `portable_simd` feature. Install with:

```sh
rustup default nightly
```

## Building & running

```sh
cargo build --release
cp .env.example .env    # then edit
./target/release/waxum
```

If `DATABASE_URL` is unset, the gateway boots against an embedded
SQLite file (`./waxum.db`) so you don't need Postgres or MySQL running
locally. Override `SQLITE_PATH` to point that file elsewhere. See the
crate-level docs (`cargo doc --open`) for the full env matrix.

## Local quality gates (required before every push)

The `.git/hooks/pre-push` hook bundled with the repo (installed on
first `git clone` — `chmod +x .git/hooks/pre-push` if it didn't come
through executable) runs the same three checks before letting a push
through, plus a `//` line-comment check (see below):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build
```

`cargo test` and `cargo audit` (advisory-database dependency scan) are
not part of the local hook, but CI (`.github/workflows/ci.yml`) runs
both on every push — run `cargo test` yourself before pushing so CI
isn't your first signal something broke.

If you're on a memory-constrained machine (this project was largely
developed on a 16GB/16-core laptop), an unbounded `cargo build`/`cargo
test` can spawn enough linker processes in parallel to exhaust memory
and produce confusing "internal compiler error"-looking failures that
are really just an OOM. Cap it if you hit that: `cargo build -j 4` /
`cargo test -j 4` (or export `CARGO_BUILD_JOBS=4`).

## Code conventions

- **`cargo fmt` is enforced** — do not commit unformatted code.
- **`clippy` is warning-clean** — `-D warnings` is the CI setting.
- **No narrative `//` comments in new code.** Doc-comments `///` and
  `//!` are encouraged; plain `//` line-comments are stripped by the
  pre-push hook.
  - Rustdoc uses `///` on items and `//!` at the top of a file/module
    to attach documentation. Prefer those when a *why* needs to be
    persisted.
  - The upside of no ambient narration: identifiers do the explaining.
    Rename first, add a doc-comment second, drop a comment last.
- **Small commits, imperative subject.** `release 0.6.9 fix foo`,
  `handlers add contact list endpoint`. The pre-push hook rejects
  messages with unquoted `+`, `-`, or `&` chars because they collide
  with the shell wrapper we use to send commits into `git`.
- **No emojis** in code, comments, commits, or user-facing strings.

## Adding an endpoint

1. Add the handler function under `src/handlers/<domain>.rs`. Use the
   existing patterns (extract JSON, resolve the client via
   [`get_live_client`](https://fdciabdul.github.io/waxum/waxum/state/struct.SessionState.html#method.get_live_client),
   `?`-propagate `ApiError`). Any new request/response type goes in
   `src/models/<domain>.rs` with `#[derive(..., ToSchema)]`.
2. Add the axum route in `src/routes/mod.rs`.
3. Register the handler on the utoipa `#[openapi(paths(…))]` list *and*
   any new request/response types on `components(schemas(…))`, both in
   `src/main.rs` — miss the schema half and the handler still shows up
   in Swagger UI, just with an incomplete/missing body schema.
4. If the endpoint touches an existing session (`{session_id}` in the
   path), it's covered by the per-session token scoping in
   `middleware::jwt::jwt_auth_middleware` automatically — no extra work
   needed unless it's a fleet-wide endpoint that should stay
   superadmin-only (see the exclusion list in
   `jwt::session_id_from_path`).
5. Update `CHANGELOG.md` under the unreleased section.

## Releasing

The release flow is manual:

1. Bump the `version` field in `Cargo.toml`.
2. Add a `## [x.y.z]` section to `CHANGELOG.md` with what changed.
3. `git commit -am "release x.y.z <short summary>"`.
4. `git push origin main` — the `release.yml` workflow tags the commit,
   builds multi-arch binaries + Docker image, and publishes the
   GitHub release.
5. On the production server: `docker pull fdciabdul/waxum:latest`, then
   `docker cp` the binary out of a temporary container and
   `pm2 restart waxum` (see the internal deploy runbook).

## Documentation

- **Rustdoc** (this repo) is published to
  <https://fdciabdul.github.io/waxum> on every push to `main`. Add
  `///` doc-comments to items you introduce so the API browser stays
  useful.
- **REST API docs** live in the separate `waxum-doc` Docusaurus repo
  and deploy to <https://waxum.imtaqin.id/docs>.

## Reporting a security issue

Don't file a public issue for a suspected vulnerability. Email
**cp@imtaqin.id** with a reproduction and, if you can, an assessment
of impact. Include the same version info as a regular bug report (see
below). A fix will get a `CHANGELOG.md` entry describing the issue
once it's out — see the "Security" section of the `0.10.0` entry for
the level of detail expected.

## Filing an issue

Use the [issue templates](.github/ISSUE_TEMPLATE/) — they prompt for
the same information listed here. If filing without one, include:

- Version (`git rev-parse HEAD` and `Cargo.toml` version).
- Backend (`DATABASE_URL` scheme is enough — don't paste creds).
- Reproduction: exact API call + observed vs expected response.
- Relevant `RUST_LOG=waxum=debug` output.

## License

By contributing you agree that your contribution is licensed under the
same MIT license that covers the rest of the project.
