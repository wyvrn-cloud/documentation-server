# documentation-server

A DIDComm v2 agent that serves DIDComm protocol documentation, the DIDComm Messaging
specification, and JSON Schemas over DIDComm itself, using the
[`https://wyvrn.app/documentation/1.1`](https://github.com/wyvrn-cloud/protocols/blob/master/protocols/documentation/1.1/readme.md)
protocol. It still answers
[`1.0`](https://github.com/wyvrn-cloud/protocols/blob/master/protocols/documentation/1.0/readme.md)
requests, in 1.0. It's the registry the [`mcp`](https://github.com/wyvrn-cloud/mcp) bridge
asks when an AI agent needs to learn a protocol on the fly.

It serves:
- **Every protocol on [didcomm.org](https://didcomm.org)**, indexed by PIURI, with:
  - metadata
  - prose sections, which requesters can ask for one at a time
  - example messages grouped by message type
  - JSON Schemas where we have them, one per DIDComm envelope style (v1 or v2)
  - which DIDComm versions it's used with (`didcomm_versions`, e.g. `["^2.0"]`), from
    its frontmatter, its source's configuration, or else its examples and schemas
  - aliases: legacy `did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/` types are accepted everywhere,
    and `mappings/piuri-paths.toml` can name more
- **Documents**: the DIDComm Messaging spec (`spec`: v2.0, v2.1 and the editor's
  draft): a table of contents, plus any single section. The table of contents lists
  every document served.
- **Hand-written JSON Schemas** for the core protocols. didcomm.org has none, so they
  live in [`schemas/`](schemas/).
- Answers to `discover-features/2.0` (it discloses `documentation/1.1` and `1.0` with
  role `registry`) and `trust-ping/2.0`.

## Running it

```sh
git submodule update --init      # the documentation sources (shallow clones)
cargo run                        # reads config/default.toml
```

It logs its DID at startup, and `GET /did` returns it. Point a requester (e.g. the MCP
bridge) at that DID.

### Container image

```sh
git submodule update --init
docker build -t documentation-server .
docker run -p 8080:8080 -v docserver-data:/app/data documentation-server
```

The volume keeps the identity, and with it the DID, across restarts. Behind a
TLS-intercepting proxy, pass its CA to the build: `--secret id=ca_bundle,src=ca.pem`. If
that proxy listens on localhost, also add `--network=host`. A `github_token` secret is
available for private git dependencies. No secret ends up in an image layer.

## Configuration

See [`config/default.toml`](config/default.toml). The settings that matter:

- **`server.public_url`**: where peers reach this server. It becomes the DIDComm
  service endpoint.
- **`server.did_method`**:
  - `peer` (the default): a `did:peer:4` derived from the server's keys. It works
    anywhere, including plain-HTTP local setups.
  - `web`: a `did:web` derived from `public_url`, e.g. `https://docs.example/` becomes
    `did:web:docs.example`. The server publishes the DID document at
    `/.well-known/did.json` and `/did.json`. `public_url` must then be the HTTPS origin
    peers resolve.
- **`identity.path`**: the server's private keys, created on first start. Keep this file
  to keep the DID.
- **`[[source]]`**: what to index.
  - The defaults are the two submodules.
  - Add any number of extra folders in didcomm.org's layout (`<name>/<version>/readme.md`).
    For example, a checkout of
    [`wyvrn-cloud/protocols`](https://github.com/wyvrn-cloud/protocols) adds our own
    protocols.
  - A source's protocols can ship JSON Schemas next to their definitions
    (`<name>/<version>/schemas/<message>.json`). Those win over [`schemas/`](schemas/).
- **`index.mappings`**: [`mappings/piuri-paths.toml`](mappings/piuri-paths.toml). It
  records didcomm.org folders whose name doesn't match their PIURI. Definitions are
  always keyed by the PIURI in their frontmatter.

## Endpoints

| | |
|---|---|
| `POST /` | DIDComm messages. A reply comes back in the HTTP response if the request asked for `return_route: "all"`; otherwise it goes to the sender's endpoint. |
| `GET /did` | This server's DID. |
| `GET /.well-known/did.json`, `GET /did.json` | Its DID document, for `did_method = "web"` (404 otherwise). |
| `GET /healthz` | Liveness. |

## Keeping the sources current

```sh
scripts/update-sources.sh
```

It moves both submodules to upstream's latest commit and re-runs the index tests. Those
fail on any new folder/PIURI mismatch (add it to the mappings file) and on any schema
that no longer matches a protocol.

## Tests

```sh
cargo test
```

- Unit tests cover the Markdown parsing, the index, and every `documentation`
  request and problem path.
- `tests/index.rs` checks the real sources:
  - everything is indexed
  - there are no warnings
  - all three spec versions are present
  - every schema compiles
  - it also prints how upstream examples fare against the schemas, and the upstream
    content it couldn't parse (11 examples that aren't even JSON5, today)
- `tests/server.rs` runs the server over HTTP with a real DIDComm client. If a checkout
  of `wyvrn-cloud/protocols` sits next to this repository, or `DOCUMENTATION_SCHEMAS`
  points at its `protocols/documentation`, every reply is also validated
  against the published schemas of its documentation version.
- `DOCSERVER_URL=http://host:port/ cargo test --test server -- --ignored` smoke-tests a
  running server or deployment.

CI (`.github/workflows/ci.yml`) runs `cargo test` and a `docker build` on every pull
request. To also validate replies against the published `documentation` schemas in
CI, add a `WYVRN_READ_TOKEN` repository secret: a token that can read
`wyvrn-cloud/protocols`.

See [`PLAN.md`](PLAN.md) for the design, and the system-wide plan in
[`mcp/PLAN.md`](https://github.com/wyvrn-cloud/mcp/blob/master/PLAN.md).
