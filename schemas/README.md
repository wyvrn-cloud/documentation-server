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

Same as the `protocols` repo's:

- [JSON Schema 2020-12](https://json-schema.org/draft/2020-12), self-contained (no
  cross-file `$ref`).
- Each `v2/` file validates a complete DIDComm v2 plaintext message: headers and `body`,
  with `type` pinned by `const`.
- `id` and `type` are always required.
- Replies require `thid`, because the spec says a message continuing a thread MUST
  carry it.
- Other headers are required where the protocol says so: `pthid` on a problem-report,
  `from` on an out-of-band invitation, `created_time` on a basicmessage.
- No `additionalProperties: false`: DIDComm recipients must ignore unknown fields.
- Wording and constraints come from the normative text, cited in each schema's
  `description`, not from the examples.

## Coverage (first pass)

| Protocol | Messages |
|---|---|
| `discover-features/2.0` | `queries`, `disclose` |
| `trust-ping/2.0` | `ping`, `ping-response` |
| `basicmessage/2.0` | `message` |
| `report-problem/2.0` | `problem-report` |
| `coordinate-mediation/3.0` | `mediate-request`, `mediate-grant`, `mediate-deny`, `recipient-update`, `recipient-update-response`, `recipient-query`, `recipient` |
| `messagepickup/3.0` | `status-request`, `status`, `delivery-request`, `delivery`, `messages-received`, `live-delivery-change` |
| `routing/2.0` | `forward` |
| `out-of-band/2.0` | `invitation` |

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
