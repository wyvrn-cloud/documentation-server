# Schemas

Hand-written JSON Schemas for protocols whose upstream definitions don't include any.
That is currently every protocol on didcomm.org. The server merges these into
`documentation/1.0` responses for the matching message types.

## Layout

```
schemas/
  v2/<PIURI host>/<protocol name>/<version>/<message-name>.json   DIDComm v2 messages
  v1/<PIURI host>/<protocol name>/<version>/<message-name>.json   DIDComm v1 messages
```

A message type used with both DIDComm v1 and v2 (several didcomm.org protocols show
examples of both) has a schema under each. The server tells them apart by what they
pin: `properties.type.const` is a v2 schema, `properties.@type` a v1 schema. It warns
when a file sits under the wrong folder. Responses list both under the message type's
`schemas`, each with its `didcomm_versions` (`^1.0` or `^2.0`).

The path comes from the **PIURI**, never from the upstream folder name. For example,
the didcomm.org folder `messagepickup/4.0` holds `https://didcomm.org/message-pickup/4.0`,
so a v2 schema for it lives under `v2/didcomm.org/message-pickup/4.0/`. A protocol known
by an alias (see `mappings/piuri-paths.toml`) is filed under the PIURI it's indexed by.

Protocols that ship their own schemas next to their `readme.md` (the
[`wyvrn-cloud/protocols`](https://github.com/wyvrn-cloud/protocols#schemas) convention)
don't need entries here. If both exist, the schema shipped with the protocol wins.

## Conventions

Shared by both styles (and by the `protocols` repo):

- [JSON Schema 2020-12](https://json-schema.org/draft/2020-12), self-contained (no
  cross-file `$ref`).
- No `additionalProperties: false`: DIDComm recipients must ignore unknown fields.
- Wording and constraints come from the normative text, cited in each schema's
  `description`, not from the examples.
- Shared pieces (decorators, the attachment descriptor) are copied into each file's
  `$defs` from [`schema-defs/`](../schema-defs/): `v1.json` and `v2.json`. A test
  fails if any copy drifts from those.

### DIDComm v2 (`v2/`)

- Each file validates a complete DIDComm v2 plaintext message: headers and `body`, with
  `type` pinned by `const`.
- `id` and `type` are always required.
- Replies require `thid`, because the spec says a message continuing a thread MUST
  carry it.
- Other headers are required where the protocol says so: `pthid` on a problem-report,
  `from` on an out-of-band invitation, `created_time` on a basicmessage, `return_route`
  where a protocol makes it mandatory.
- Adopted messages (`problem-report`, `ack`) reuse the spec's definitions under the
  adopting protocol's type.

### DIDComm v1 (`v1/`)

- Each file validates a complete DIDComm v1 (Aries) plaintext message: the protocol's
  fields at the top level, plus `@id`, `@type` and decorators.
- `@type` is pinned with `enum`: the `https://didcomm.org/` type and its legacy
  `did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/` equivalent
  ([Aries RFC 0348](https://github.com/hyperledger/aries-rfcs/tree/main/features/0348-transition-msg-type-to-https)).
- `@id` and `@type` are always required.
- Replies require `~thread` with `thid`
  ([Aries RFC 0008](https://github.com/hyperledger/aries-rfcs/tree/main/concepts/0008-message-id-and-threading):
  "generally required on any type of response").
- Every schema types the common decorators when present: `~thread`, `~l10n`,
  `~timing`, `~transport`, `~please_ack`, `~service`, `~trace`; `*~attach` fields use
  the RFC 0017 attachment descriptor. A protocol that makes a decorator mandatory
  (`~transport.return_route` in pickup, `~thread.pthid` on a DID Exchange request)
  requires it.
- Adopted messages (`ack`, `problem-report`) follow RFCs 0015 and 0035 under the
  adopting protocol's type.

## Coverage

Stalled and Retired Aries RFCs are documented but get no schemas.

### Tier 1: Production (didcomm.org) and Adopted / Accepted (Aries RFCs)

| Protocol | Style | Messages |
|---|---|---|
| `discover-features/2.0` | v2, v1 | `queries`, `disclose`; v1: `queries`, `disclosures` |
| `discover-features/1.0` | v1 | `query`, `disclose` |
| `trust-ping/2.0`, `trust_ping/1.0` | v2, v1 | `ping`, `ping-response` / `ping_response` |
| `basicmessage/2.0`, `basicmessage/1.0` | v2 + v1, v1 | `message` |
| `report-problem/2.0`, `report-problem/1.0`, `notification/1.0` | v2, v1, v1 | `problem-report`; `ack` |
| `coordinate-mediation/3.0`, `2.0`, `1.0` | v2, v2, v1 | mediate request/grant/deny, keylist or recipient update/query |
| `messagepickup/3.0`, `message-pickup/4.0`, `messagepickup/2.0` | v2, v2 + v1, v1 | status, delivery, live mode, problem-report |
| `routing/2.0` | v2 | `forward` |
| `out-of-band/2.0`, `out-of-band/1.1` | v2, v1 | `invitation`; v1 also `handshake-reuse`, `handshake-reuse-accepted`, `problem_report` |
| `didexchange/1.1`, `1.0` | v1 | `request`, `response`, `complete`, `problem_report` |
| `connections/1.0` | v1 | `invitation`, `request`, `response`, `problem_report` |
| `issue-credential/2.0`, `1.1`, `1.0` | v1 | propose, offer, request, issue, `ack`, `problem-report` |
| `present-proof/2.0`, `1.0` | v1 | propose, request, presentation, `ack`, `problem-report` |
| `revocation_notification/2.0`, `1.0` | v1 | `revoke` (2.0: also `unrevoke`) |
| `did-rotate/1.0` | v1 | `rotate`, `ack`, `problem-report`, `hangup` |
| `action-menu/2.0`, `1.0` | v2, v1 | `menu`, `menu-request`, `perform` |
| `questionanswer/1.0` | v2 + v1 | `question`, `answer` |
| `data-agreement-negotiation/1.0`, `-proofs/1.0`, `-termination/1.0` | v2 | offer/accept/reject, verify request/response, terminate/ack, problem-report |

`data-disclosure-agreement/1.0` has no schemas: its examples use other PIURIs
(`dda/1.0`, `dda-negotiation/1.0`) than its own. `data-agreement-context-decorator/1.0`
defines a decorator, not messages.

## Validation notes (2026-10-01)

- All schemas pass the JSON Schema 2020-12 meta-schema.
- They were checked against **real traffic** with the Indicio public mediator
  (`did:web:us-east2.public.mediator.indiciotech.io`). All six messages we sent and all
  six replies (`ping-response`, `disclose`, `mediate-grant`, two `status`, `recipient`)
  validate.
- **Upstream examples:** 17 of the 37 examples on didcomm.org and in the v2.1 spec
  that these schemas cover validate. The other 20 are shortcuts in the examples, not
  schema mistakes:
  - Placeholders instead of values (5), e.g. `"<did for messages>"`, `"<sender's did>"`,
    or an empty `attachments` array.
  - A missing `id` (11), which the spec makes REQUIRED.
  - A reply with no `thid` (4).
  - `poe/1.0`'s example additionally leaves out `feature-type` in its disclosures.

  Upstream examples are served as written, so the server's example-vs-schema check
  must report these, not fail on them.

## Validation notes, tier 1 (2026-10-02)

- Every schema compiles, and the shared `$defs` match `schema-defs/`.
- **Upstream examples:** 121 examples (v1 and v2) are checked against the schema for
  their envelope style; 68 don't validate. All of them are shortcuts in the examples:
  - Placeholder message types the RFCs write instead of a version (15: `%VER`,
    `<baseuri>`), and examples showing another version's types (9, e.g. DID Exchange
    1.1's examples under 1.0).
  - Placeholder values instead of DIDs (11, e.g. `"<did for messages>"`).
  - A missing `@id` (10), or a reply with no `~thread` / `thid` (14).
  - The data-agreement pages' `to` written as a string, not an array (7).
  - did-rotate's `ack` without the `status` RFC 0015 requires, and RFC 0035's sample
    with a placeholder string for `~thread`.
