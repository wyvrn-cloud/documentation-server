# Plan: documentation-server

A DIDComm v2 agent that serves DIDComm protocol documentation, the DIDComm Messaging
spec, and hand-written JSON Schemas over
[`https://wyvrn.app/documentation/1.0`](https://github.com/wyvrn-cloud/protocols).
For the whole-system plan (MCP bridge, decisions, phases) see
[`mcp/PLAN.md`](https://github.com/wyvrn-cloud/mcp/blob/master/PLAN.md); this file
covers only this repo.

## Layout

```
sources/
  didcomm.org/          git submodule: decentralized-identity/didcomm.org (shallow)
  didcomm-messaging/    git submodule: decentralized-identity/didcomm-messaging (shallow)
schemas/                hand-written JSON Schemas, keyed by PIURI
  didcomm.org/<name>/<version>/<message>.json
mappings/
  piuri-paths.toml      PIURI → source path mapping until upstream naming is fixed
config/
  default.toml          default sources and server settings
src/                    Rust: indexer, DIDComm handler, HTTP server
```

The `didcomm-messaging` repo is about 95 MB, about 90 MB of it images under
`docs/collateral*`. The submodule is marked `shallow = true` and pinned to a commit.
`scripts/update-sources.sh` bumps both submodules.

## Sources are configurable

```toml
[[source]]
kind = "protocol-registry"        # didcomm.org layout: <dir>/<name>/<version>/readme.md
path = "sources/didcomm.org/site/content/protocols"

[[source]]
kind = "didcomm-spec"             # reads specs.json for markdown_paths per version
path = "sources/didcomm-messaging"

# Optional, off by default -- e.g. a checkout of wyvrn-cloud/protocols:
# [[source]]
# kind = "protocol-registry"
# path = "/srv/wyvrn-protocols/protocols"
# schemas = "/srv/wyvrn-protocols/schemas"
```

Each `protocol-registry` source may have its own `schemas` overlay folder in addition
to the repo-level `schemas/`.

## Indexing (at startup; ~60 files, so no build step is needed)

- **Protocols:** read each `readme.md`'s YAML frontmatter and key it by its `piuri`.
  `mappings/piuri-paths.toml` takes precedence; a test fails if a frontmatter PIURI
  disagrees with its folder and has no mapping entry, so new mismatches can't slip in.
  Split the body into sections by heading (ids are lower-kebab-case headings, as
  `documentation/1.0` defines). Pull JSON examples out of fenced code blocks and group
  them by `type` (v2) or `@type` (v1). Parse them leniently (JSON5), because several
  upstream examples have trailing commas or `//` comments. Note the roles and states
  tables.
- **Spec:** for each spec version in `specs.json` (editor's draft, `v2.0`, `v2.1`
  snapshots), stitch the `markdown_paths` in order and split by heading into
  addressable sections with a table of contents.
- **Schemas:** load `<protocol dir>/schemas/*.json` shipped next to a definition (the
  `wyvrn-cloud/protocols` convention), then the overlay folders (`schemas/<PIURI path>/`),
  and attach them by the `type` const. A schema shipped with the protocol wins over an
  overlay. A test validates every extracted example against its schema and *reports*
  mismatches without failing, because upstream examples legitimately use placeholders
  and skip required headers (see `schemas/README.md`). It does fail if one of our own
  schemas is invalid, or a `documentation/1.0` example doesn't validate.

## DIDComm endpoint

- axum over HTTP(S), `POST /` for DIDComm messages. Replies are synchronous when the
  sender asks for `return_route: all`, otherwise packed and sent to the sender's
  service endpoint.
- Identity: persistent keys, published as `did:web` (the server serves its own
  `/.well-known/did.json`), and as `did:peer:4` for local/dev use.
- Handles `documentation/1.0` (`query`, `request`, `spec-request`),
  `discover-features/2.0` (advertising `documentation/1.0`) and `trust-ping/2.0`.
  Uses `report-problem/2.0` for errors.
- Built on the shared agent crate from the `didcomm` repo (see the system plan,
  phase 2).
- Ships as a Dockerfile; the image includes the pinned submodule contents.

## First-pass schemas

discover-features 2.0, trust-ping 2.0, basicmessage 2.0, report-problem 2.0,
coordinate-mediation 3.0, messagepickup 3.0, routing 2.0, out-of-band 2.0, and
documentation 1.0 (that one lives with its spec in `wyvrn-cloud/protocols`).
