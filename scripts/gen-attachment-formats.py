#!/usr/bin/env python3
"""Generate attachment-formats/: formats.toml and one JSON Schema per
(format, message), then validate every example payload the RFCs and WACI show."""
import json, os, re, sys, base64
from pathlib import Path
import json5
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "attachment-formats"
DRAFT = "https://json-schema.org/draft/2020-12/schema"

IC = ["https://didcomm.org/issue-credential/2.0", "https://didcomm.org/issue-credential/3.0"]
IC2 = IC[:1]
IC3 = IC[1:]
PP = ["https://didcomm.org/present-proof/2.0", "https://didcomm.org/present-proof/3.0"]
PP2 = PP[:1]
PP3 = PP[1:]

# ---- shared building blocks -------------------------------------------------------
STR = {"type": "string"}
STR_OR_ARR = {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}, "minItems": 1}]}
CONTEXT = {"description": "JSON-LD context.", "oneOf": [{"type": "string"}, {"type": "object"}, {"type": "array", "minItems": 1}]}
PROOF = {"description": "One proof or a set of them.", "oneOf": [{"type": "object", "required": ["type"]}, {"type": "array", "minItems": 1, "items": {"type": "object", "required": ["type"]}}]}
ISSUER = {"oneOf": [{"type": "string"}, {"type": "object", "required": ["id"], "properties": {"id": STR}}]}

def vc(proof_required, offer=False):
    """A W3C Verifiable Credential (data model 1.1 or 2.0), loosely: the parts every
    credential has. offer=True allows what an offer may leave out (issuer, dates)."""
    required = ["@context", "type", "credentialSubject"] + ([] if offer else ["issuer"]) + (["proof"] if proof_required else [])
    s = {
        "type": "object",
        "required": required,
        "properties": {
            "@context": CONTEXT,
            "id": STR,
            "type": STR_OR_ARR,
            "issuer": ISSUER,
            "issuanceDate": STR, "expirationDate": STR, "validFrom": STR, "validUntil": STR,
            "credentialSubject": {"oneOf": [{"type": "object"}, {"type": "array", "items": {"type": "object"}, "minItems": 1}]},
            "credentialStatus": {"type": "object"},
        },
    }
    if proof_required:
        s["properties"]["proof"] = PROOF
    else:
        s["not"] = {"required": ["proof"]}
    return s

PRESENTATION_DEFINITION = {
    "description": "A DIF Presentation Exchange presentation definition.",
    "type": "object",
    "required": ["input_descriptors"],
    "properties": {
        "id": STR, "name": STR, "purpose": STR,
        "format": {"type": "object"},
        "submission_requirements": {"type": "array", "items": {"type": "object"}},
        "frame": {"type": "object"},
        "input_descriptors": {"type": "array", "items": {
            "type": "object", "required": ["id"],
            "properties": {"id": STR, "name": STR, "purpose": STR, "group": {"type": "array", "items": STR},
                           "schema": {}, "format": {"type": "object"}, "constraints": {"type": "object"}},
        }},
    },
}
DESCRIPTOR_MAP = {"type": "array", "items": {"type": "object", "required": ["id", "path"],
                  "properties": {"id": STR, "path": STR, "format": STR, "path_nested": {"type": "object"}}}}
PRESENTATION_SUBMISSION = {"type": "object", "required": ["descriptor_map"],
                           "properties": {"id": STR, "definition_id": STR, "descriptor_map": DESCRIPTOR_MAP}}
VP_WITH_SUBMISSION = {
    "description": "A Verifiable Presentation carrying a presentation_submission.",
    "type": "object",
    "required": ["presentation_submission"],
    "properties": {
        "@context": CONTEXT, "type": STR_OR_ARR, "holder": STR,
        "presentation_submission": PRESENTATION_SUBMISSION,
        "verifiableCredential": {"type": "array"},
        "proof": PROOF,
    },
}
PE_OPTIONS = {"type": "object", "properties": {"challenge": STR, "domain": STR}}

INDY_CRED_FILTER = {"schema_issuer_did": STR, "schema_name": STR, "schema_version": STR, "schema_id": STR, "issuer_did": STR, "cred_def_id": STR}
ANON_CRED_FILTER = {"schema_issuer_id": STR, "schema_name": STR, "schema_version": STR, "schema_id": STR, "issuer_id": STR, "cred_def_id": STR}
OFFER = {"type": "object", "required": ["schema_id", "cred_def_id", "nonce", "key_correctness_proof"],
         "properties": {"schema_id": STR, "cred_def_id": STR, "nonce": STR, "key_correctness_proof": {"type": "object"}, "method_name": STR}}
CRED_REQ_PROPS = {"prover_did": STR, "entropy": STR, "cred_def_id": STR, "blinded_ms": {"type": "object"},
                  "blinded_ms_correctness_proof": {"type": "object"}, "nonce": STR}
CRED = {"type": "object", "required": ["schema_id", "cred_def_id", "values", "signature", "signature_correctness_proof"],
        "properties": {
            "schema_id": STR, "cred_def_id": STR, "rev_reg_id": {"type": ["string", "null"]},
            "values": {"type": "object", "additionalProperties": {"type": "object", "required": ["raw", "encoded"],
                       "properties": {"raw": STR, "encoded": {"type": "string", "pattern": "^-?[0-9]+$"}}}},
            "signature": {"type": "object"}, "signature_correctness_proof": {"type": "object"},
            "rev_reg": {"type": ["object", "null"]}, "witness": {"type": ["object", "null"]}}}
NON_REVOKED = {"type": "object", "properties": {"from": {"type": "integer"}, "to": {"type": "integer"}}}
RESTRICTIONS = {"type": "array", "items": {"type": "object"}}

def proof_request(nonce_required):
    return {
        "type": "object",
        "required": ["name", "version", "requested_attributes", "requested_predicates"] + (["nonce"] if nonce_required else []),
        "properties": {
            "name": STR, "version": STR,
            "nonce": {"type": "string", "pattern": "^[0-9]+$", "description": "A decimal number, at least 80 bits."},
            "requested_attributes": {"type": "object", "additionalProperties": {
                "type": "object",
                "oneOf": [{"required": ["name"], "not": {"required": ["names"]}}, {"required": ["names"], "not": {"required": ["name"]}}],
                "properties": {"name": STR, "names": {"type": "array", "items": STR, "minItems": 1},
                               "restrictions": RESTRICTIONS, "non_revoked": NON_REVOKED}}},
            "requested_predicates": {"type": "object", "additionalProperties": {
                "type": "object", "required": ["name", "p_type", "p_value"],
                "properties": {"name": STR, "p_type": {"enum": [">=", ">", "<=", "<"]}, "p_value": {"type": "integer"},
                               "restrictions": RESTRICTIONS, "non_revoked": NON_REVOKED}}},
            "non_revoked": NON_REVOKED,
            "ver": {"enum": ["1.0", "2.0"]},
        },
    }

PROOF_INDY = {"type": "object", "required": ["proof", "requested_proof", "identifiers"],
              "properties": {
                  "proof": {"type": "object", "required": ["proofs", "aggregated_proof"]},
                  "requested_proof": {"type": "object", "properties": {
                      "revealed_attrs": {"type": "object"}, "revealed_attr_groups": {"type": "object"},
                      "self_attested_attrs": {"type": "object"}, "unrevealed_attrs": {"type": "object"},
                      "predicates": {"type": "object"}}},
                  "identifiers": {"type": "array", "items": {"type": "object", "required": ["schema_id", "cred_def_id"],
                                  "properties": {"schema_id": STR, "cred_def_id": STR, "rev_reg_id": {"type": ["string", "null"]},
                                                 "timestamp": {"type": ["integer", "null"]}}}}}}

DIDCOMM_SIGNED_ATTACHMENT_METHOD = {"type": "object", "required": ["algs_supported", "did_methods_supported", "nonce"],
    "properties": {"algs_supported": {"type": "array", "items": STR, "minItems": 1},
                   "did_methods_supported": {"type": "array", "items": STR, "minItems": 1}, "nonce": STR}}
ANONCREDS_LINK_SECRET_METHOD = {"type": "object", "required": ["nonce", "cred_def_id", "key_correctness_proof"],
    "properties": {"nonce": STR, "cred_def_id": STR, "key_correctness_proof": {}}}
SIGNED_ATTACHMENT_PROOF = {"type": "object", "required": ["attachment_id"], "properties": {"attachment_id": STR}}
BINDING = {"binding_required": {"type": "boolean"}}
BINDING_IF = {"if": {"properties": {"binding_required": {"const": True}}, "required": ["binding_required"]}, "then": {"required": ["binding_method"]}}

def obj(required, props, **extra):
    s = {"type": "object", "required": required, "properties": props}
    s.update(extra)
    return s

SD_JWT = "^[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+(~[A-Za-z0-9_.-]*)*~?$"

# ---- the formats ------------------------------------------------------------------
# (id, title, document, section, [(protocols, message, schema)])
D_AF = "aries/attachment-formats"
FORMATS = [
    ("hlindy/cred-filter@v2.0", "Indy credential filter", D_AF, "rfc0592", [(IC2, "propose-credential", obj([], INDY_CRED_FILTER))]),
    ("hlindy/cred-abstract@v2.0", "Indy credential offer (abstract)", D_AF, "rfc0592", [(IC2, "offer-credential", OFFER)]),
    ("hlindy/cred-req@v2.0", "Indy credential request", D_AF, "rfc0592", [(IC2, "request-credential",
        obj(["prover_did", "cred_def_id", "blinded_ms", "blinded_ms_correctness_proof", "nonce"], CRED_REQ_PROPS))]),
    ("hlindy/cred@v2.0", "Indy credential", D_AF, "rfc0592", [(IC2, "issue-credential", CRED)]),
    ("hlindy/proof-req@v2.0", "Indy proof request", D_AF, "rfc0592", [
        (PP2, "propose-presentation", proof_request(False)), (PP2, "request-presentation", proof_request(True))]),
    ("hlindy/proof@v2.0", "Indy proof", D_AF, "rfc0592", [(PP2, "presentation", PROOF_INDY)]),

    ("anoncreds/credential-filter@v1.0", "AnonCreds credential filter", D_AF, "rfc0771", [(IC, "propose-credential", obj([], ANON_CRED_FILTER))]),
    ("anoncreds/credential-offer@v1.0", "AnonCreds credential offer", D_AF, "rfc0771", [(IC, "offer-credential", OFFER)]),
    ("anoncreds/credential-request@v1.0", "AnonCreds credential request", D_AF, "rfc0771", [(IC, "request-credential",
        obj(["cred_def_id", "blinded_ms", "blinded_ms_correctness_proof", "nonce"], CRED_REQ_PROPS,
            anyOf=[{"required": ["entropy"]}, {"required": ["prover_did"]}]))]),
    ("anoncreds/credential@v1.0", "AnonCreds credential", D_AF, "rfc0771", [(IC, "issue-credential", CRED)]),
    ("anoncreds/proof-request@v1.0", "AnonCreds proof request", D_AF, "rfc0771", [
        (PP, "propose-presentation", proof_request(False)), (PP, "request-presentation", proof_request(True))]),
    ("anoncreds/proof@v1.0", "AnonCreds proof", D_AF, "rfc0771", [(PP, "presentation", PROOF_INDY)]),

    ("aries/ld-proof-vc-detail@v1.0", "JSON-LD credential detail (Linked Data Proof)", D_AF, "rfc0593", [
        (IC, m, obj(["credential", "options"], {
            "credential": vc(False),
            "options": obj(["proofType"], {"proofType": STR, "proofPurpose": STR, "created": STR, "challenge": STR, "domain": STR,
                                           "credentialStatus": obj(["type"], {"type": STR})})}))
        for m in ["propose-credential", "offer-credential", "request-credential"]]),
    ("aries/ld-proof-vc@v1.0", "JSON-LD verifiable credential (Linked Data Proof)", D_AF, "rfc0593", [(IC, "issue-credential", vc(True))]),

    ("didcomm/w3c-di-vc-offer@v0.1", "W3C VC (Data Integrity) offer", D_AF, "rfc0809", [(IC, "offer-credential", obj(
        ["data_model_versions_supported", "credential"],
        {"data_model_versions_supported": {"type": "array", "items": {"enum": ["1.1", "2.0"]}, "minItems": 1},
         **BINDING,
         "binding_method": {"type": "object", "properties": {"anoncreds_link_secret": ANONCREDS_LINK_SECRET_METHOD,
                                                             "didcomm_signed_attachment": DIDCOMM_SIGNED_ATTACHMENT_METHOD}},
         "credential": vc(False, offer=True)}, **BINDING_IF))]),
    ("didcomm/w3c-di-vc-request@v0.1", "W3C VC (Data Integrity) request", D_AF, "rfc0809", [(IC, "request-credential", obj(
        ["data_model_version"],
        {"data_model_version": {"enum": ["1.1", "2.0"]},
         "binding_proof": {"type": "object", "properties": {
             "anoncreds_link_secret": obj(["entropy", "blinded_ms", "nonce"], {"entropy": STR, "cred_def_id": STR,
                 "blinded_ms": {"type": "object"}, "blinded_ms_correctness_proof": {"type": "object"},
                 "blinded_ms_corectness_proof": {"type": "object", "description": "The RFC's own misspelling, in its examples."}, "nonce": STR}),
             "didcomm_signed_attachment": SIGNED_ATTACHMENT_PROOF}}}))]),
    ("didcomm/w3c-di-vc@v0.1", "W3C VC (Data Integrity)", D_AF, "rfc0809", [(IC, "issue-credential", obj(["credential"], {"credential": vc(True)}))]),

    ("didcomm/w3c-vc-sd-jwt-offer@v1.0", "W3C VC (SD-JWT) offer", D_AF, "rfc0881", [(IC, "offer-credential", obj(
        ["credential"],
        {**BINDING,
         "binding_method": {"type": "object", "properties": {"didcomm_signed_attachment": DIDCOMM_SIGNED_ATTACHMENT_METHOD}},
         "selectively_disclosable_claims": {"type": "array", "items": {"type": "string", "pattern": "^\\$(\\.[A-Za-z_@][A-Za-z0-9_@-]*|\\[[0-9]+\\])+$"}},
         "credential": vc(False, offer=True)}, **BINDING_IF))]),
    ("didcomm/w3c-vc-sd-jwt-request@v1.0", "W3C VC (SD-JWT) request", D_AF, "rfc0881", [(IC, "request-credential", obj(
        [], {"binding_proof": {"type": "object", "properties": {"didcomm_signed_attachment": SIGNED_ATTACHMENT_PROOF}}}))]),
    ("didcomm/w3c-vc-sd-jwt@v1.0", "W3C VC (SD-JWT)", D_AF, "rfc0881", [(IC, "issue-credential", obj(
        ["credential"], {"credential": {"type": "string", "pattern": SD_JWT, "description": "The SD-JWT, in compact serialization."}}))]),

    ("dif/presentation-exchange/definitions@v1.0", "DIF Presentation Exchange definition", D_AF, "rfc0510", [
        (PP, "propose-presentation", PRESENTATION_DEFINITION),
        (PP, "request-presentation", obj(["presentation_definition"], {"options": PE_OPTIONS, "presentation_definition": PRESENTATION_DEFINITION}))]),
    ("dif/presentation-exchange/submission@v1.0", "DIF Presentation Exchange submission", D_AF, "rfc0510", [(PP, "presentation", VP_WITH_SUBMISSION)]),

    ("dif/credential-manifest@v1.0", "DIF Credential Manifest", D_AF, "rfc0511", [
        (IC2, "propose-credential", obj([], {"issuer": STR, "credential": obj([], {"name": STR, "schema": STR})})),
        (IC2, "offer-credential", obj(["challenge", "domain", "credential_manifest"], {"challenge": STR, "domain": STR, "credential_manifest": {"type": "object"}})),
        (IC2, "request-credential", obj([], {"credential-manifest": {"type": "object"}, "presentation-submission": VP_WITH_SUBMISSION},
                                         not_={"required": ["credential-manifest", "presentation-submission"]})),
    ]),
    ("dif/credential-manifest/manifest@v1.0", "DIF Credential Manifest", "waci-didcomm", None, [(IC3, "offer-credential", obj(
        ["credential_manifest"], {"options": PE_OPTIONS, "credential_manifest": obj(["id", "issuer", "output_descriptors"], {
            "id": STR, "version": STR, "issuer": obj(["id"], {"id": STR, "name": STR, "styles": {"type": "object"}}),
            "output_descriptors": {"type": "array", "items": obj(["id", "schema"], {"id": STR, "schema": STR})},
            "presentation_definition": PRESENTATION_DEFINITION, "format": {"type": "object"}})}))]),
    ("dif/credential-manifest/application@v1.0", "DIF Credential Application", "waci-didcomm", None, [(IC3, "request-credential", obj(
        ["credential_application"], {"@context": CONTEXT, "type": STR_OR_ARR,
            "credential_application": obj(["id", "manifest_id"], {"id": STR, "manifest_id": STR, "format": {"type": "object"}}),
            "presentation_submission": PRESENTATION_SUBMISSION, "verifiableCredential": {"type": "array"}, "proof": PROOF}))]),
    ("dif/credential-manifest/fulfillment@v1.0", "DIF Credential Fulfillment", "waci-didcomm", None, [(IC3, "issue-credential", obj(
        ["credential_fulfillment", "verifiableCredential"], {"@context": CONTEXT, "type": STR_OR_ARR,
            "credential_fulfillment": obj(["id", "manifest_id", "descriptor_map"], {"id": STR, "manifest_id": STR, "descriptor_map": DESCRIPTOR_MAP}),
            "verifiableCredential": {"type": "array", "minItems": 1}, "proof": PROOF}))]),
]

# DIDComm v1 (2.0 protocols): the decorator each message carries its formatted attachments in.
V1_ATTACHMENTS = {
    "https://didcomm.org/issue-credential/2.0/propose-credential": "filters~attach",
    "https://didcomm.org/issue-credential/2.0/offer-credential": "offers~attach",
    "https://didcomm.org/issue-credential/2.0/request-credential": "requests~attach",
    "https://didcomm.org/issue-credential/2.0/issue-credential": "credentials~attach",
    "https://didcomm.org/present-proof/2.0/propose-presentation": "proposals~attach",
    "https://didcomm.org/present-proof/2.0/request-presentation": "request_presentations~attach",
    "https://didcomm.org/present-proof/2.0/presentation": "presentations~attach",
}

def fix(schema):
    """not_ -> not (a Python keyword workaround), recursively."""
    if isinstance(schema, dict):
        return {("not" if k == "not_" else k): fix(v) for k, v in schema.items()}
    if isinstance(schema, list):
        return [fix(v) for v in schema]
    return schema

def toml_str(s):
    return json.dumps(s)

def main():
    if OUT.exists():
        for p in sorted(OUT.rglob("*.json")):
            p.unlink()
    OUT.mkdir(exist_ok=True)
    lines = [
        "# Attachment formats: the payloads issue-credential and present-proof carry in their",
        "# attachments, named by a format identifier (a DIDComm v1 message's formats[].format, a",
        "# v2 attachment's format). Generated; see README.md. For each [[format]] and each of its",
        "# messages, <id>/<message>.json is the JSON Schema of the attachment's content in that",
        "# message, served with every protocol listed.",
        "",
        "# DIDComm v1: the attachment decorator of each message that carries formatted attachments.",
        "[attachments]",
    ]
    for message, decorator in V1_ATTACHMENTS.items():
        lines.append(f"{toml_str(message)} = {toml_str(decorator)}")
    count = 0
    validators = {}
    for fid, title, document, section, uses in FORMATS:
        lines += ["", "[[format]]", f"id = {toml_str(fid)}", f"title = {toml_str(title)}", f"document = {toml_str(document)}"]
        if section:
            lines.append(f"section = {toml_str(section)}")
        for protocols, message, schema in uses:
            lines += ["[[format.use]]", f"message = {toml_str(message)}", "protocols = [" + ", ".join(toml_str(p) for p in protocols) + "]"]
            full = {"$schema": DRAFT, "$id": f"https://docs.wyvrn.app/attachment-formats/{fid}/{message}.json",
                    "title": f"{title} ({fid}) in {message}", **fix(schema)}
            Draft202012Validator.check_schema(full)
            path = OUT / fid / f"{message}.json"
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(full, indent=2) + "\n")
            count += 1
            for p in protocols:
                validators[(fid, f"{p}/{message}")] = Draft202012Validator(full)
    (OUT / "formats.toml").write_text("\n".join(lines) + "\n")
    print(f"{len(FORMATS)} formats, {count} schemas")
    check_examples(validators)

FENCE = re.compile(r"```[a-zA-Z]*\n(.*?)```", re.S)

def payloads(message):
    """(format, message type, attachment content) for every attachment with inline JSON."""
    mtype = message.get("@type") or message.get("type")
    if not isinstance(mtype, str):
        return
    mtype = mtype.replace("%VER", "")
    if "@type" in message:  # v1: formats[] names attach ids
        by_id = {f.get("attach_id"): f.get("format") for f in message.get("formats", []) if isinstance(f, dict)}
        for key, value in message.items():
            if key.endswith("~attach") and isinstance(value, list):
                for a in value:
                    if isinstance(a, dict) and isinstance(a.get("data", {}).get("json"), (dict, list)):
                        yield by_id.get(a.get("@id")), mtype, a["data"]["json"]
    else:
        for a in message.get("attachments", []) or []:
            if isinstance(a, dict) and isinstance(a.get("data", {}).get("json"), (dict, list)):
                yield a.get("format"), mtype, a["data"]["json"]

def check_examples(validators):
    sources = [ROOT / "sources/aries-rfcs/features" / d / "README.md" for d in [
        "0453-issue-credential-v2", "0454-present-proof-v2", "0510-dif-pres-exch-attach", "0511-dif-cred-manifest-attach",
        "0592-indy-attachments", "0593-json-ld-cred-attach", "0771-anoncreds-attachments",
        "0809-w3c-data-integrity-credential-attachment", "0881-w3c-vc-sd-jwt-credential-attachment"]]
    sources += [ROOT / "sources/waci-didcomm/spec/v1.0/spec.md", ROOT / "sources/waci-didcomm/issue_credential/README.md",
                ROOT / "sources/waci-didcomm/present_proof/present-proof-v3.md"]
    checked = failed = 0
    for src in sources:
        for block in FENCE.findall(src.read_text()):
            try:
                message = json5.loads(block)
            except ValueError:
                continue
            if not isinstance(message, dict):
                continue
            for fmt, mtype, content in payloads(message):
                # Message types written with %VER: try the versions the format is used with.
                keys = [k for k in validators if k[0] == fmt and k[1].rsplit("/", 1)[1] == mtype.rsplit("/", 1)[1]]
                if not keys:
                    if fmt and "<" not in fmt:
                        print(f"  no schema for {fmt} in {mtype} ({src.name})")
                    continue
                v = validators[keys[0]]
                errors = list(v.iter_errors(content))
                checked += 1
                if errors:
                    failed += 1
                    print(f"  MISMATCH {fmt} in {mtype} ({src.parent.name}): {errors[0].message[:160]} at {list(errors[0].absolute_path)}")
    # The RFCs' standalone "might look like this" payloads.
    for fid, message, src, marker in STANDALONE:
        text = (ROOT / src).read_text()
        i = text.index(marker)
        block = FENCE.search(text, i).group(1)
        content = json5.loads(block)
        v = next(v for k, v in validators.items() if k[0] == fid and k[1].endswith("/" + message))
        errors = list(v.iter_errors(content))
        checked += 1
        if errors:
            failed += 1
            print(f"  MISMATCH {fid} in {message} (standalone in {src}): {errors[0].message[:160]} at {list(errors[0].absolute_path)}")
    print(f"{checked} example payloads checked, {failed} mismatched")

R = "sources/aries-rfcs/features/"
STANDALONE = [
    ("hlindy/cred-filter@v2.0", "propose-credential", R + "0592-indy-attachments/README.md", "For example, the JSON (non-base64-encoded) structure"),
    ("anoncreds/credential-filter@v1.0", "propose-credential", R + "0771-anoncreds-attachments/README.md", "For example, the JSON structure might look like this"),
    ("aries/ld-proof-vc-detail@v1.0", "request-credential", R + "0593-json-ld-cred-attach/README.md", "The JSON structure might look like this"),
    ("aries/ld-proof-vc@v1.0", "issue-credential", R + "0593-json-ld-cred-attach/README.md", "Format identifier: `aries/ld-proof-vc@v1.0`"),
    ("didcomm/w3c-di-vc-offer@v0.1", "offer-credential", R + "0809-w3c-data-integrity-credential-attachment/README.md", "Format identifier: `didcomm/w3c-di-vc-offer@v0.1`"),
    ("didcomm/w3c-di-vc-request@v0.1", "request-credential", R + "0809-w3c-data-integrity-credential-attachment/README.md", "Format identifier: `didcomm/w3c-di-vc-request@v0.1`"),
    ("didcomm/w3c-vc-sd-jwt-offer@v1.0", "offer-credential", R + "0881-w3c-vc-sd-jwt-credential-attachment/README.md", "Format identifier: `didcomm/w3c-vc-sd-jwt-offer@v1.0`"),
    ("didcomm/w3c-vc-sd-jwt-request@v1.0", "request-credential", R + "0881-w3c-vc-sd-jwt-credential-attachment/README.md", "Format identifier: `didcomm/w3c-vc-sd-jwt-request@v1.0`"),
]

if __name__ == "__main__":
    main()
