//! The index over the real sources (the submodules) with the repository's own
//! `config/default.toml`. Needs `git submodule update --init`.

use documentation_server::{
    config::Config,
    index::{Envelope, Index},
};

fn index() -> Index {
    Index::build(&Config::load("config/default.toml").unwrap())
}

#[test]
fn indexes_everything_without_warnings() {
    let index = index();
    assert!(index.warnings.is_empty(), "index warnings:\n{}", index.warnings.join("\n"));
    eprintln!("{} upstream content issues:", index.upstream_issues.len());
    for issue in &index.upstream_issues {
        eprintln!("  {issue}");
    }

    let readmes = std::fs::read_dir("sources/didcomm.org/site/content/protocols")
        .expect("submodules checked out (git submodule update --init)")
        .flatten()
        .flat_map(|name| std::fs::read_dir(name.path()).into_iter().flatten().flatten())
        .filter(|version| version.path().join("readme.md").is_file())
        .count();
    assert!(readmes >= 50, "expected the full didcomm.org registry, found {readmes} definitions");
    let from_didcomm_org = index.protocols.values().filter(|d| d.source.name == "didcomm.org").count();
    let from_aries = index.protocols.values().filter(|d| d.source.name == "aries-rfcs").count();
    let from_waci = index.protocols.values().filter(|d| d.source.name == "waci-didcomm").count();
    // Every didcomm.org page is indexed, except stubs an RFC replaced.
    let replaced = readmes - from_didcomm_org;
    assert!((10..=20).contains(&replaced), "{replaced} didcomm.org stubs replaced");
    assert!(from_aries >= 40, "{from_aries} protocols from the Aries RFCs");
    assert_eq!(from_waci, 2);
    assert_eq!(index.protocols.len(), from_didcomm_org + from_aries + from_waci);
}

#[test]
fn aries_rfcs_define_the_didcomm_v1_protocols() {
    let index = index();
    // A didcomm.org page that only links to the RFC is replaced by the RFC.
    let exchange = &index.protocols["https://didcomm.org/didexchange/1.1"];
    assert_eq!(exchange.source.name, "aries-rfcs");
    assert_eq!(exchange.source.path, "features/0023-did-exchange/README.md");
    assert_eq!(exchange.status, "Adopted");
    assert_eq!(exchange.didcomm_versions(), ["^1.0"]);
    assert!(exchange.aliases.contains(&"did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/didexchange/1.1".to_string()));
    let types: Vec<_> = exchange.messages.iter().map(|m| m.message_type.as_str()).collect();
    for name in ["request", "response", "complete"] {
        assert!(types.contains(&format!("https://didcomm.org/didexchange/1.1/{name}").as_str()), "{types:?}");
    }
    // Placeholders in the RFC's types are filled in.
    let oob = &index.protocols["https://didcomm.org/out-of-band/1.1"];
    assert!(oob.message("https://didcomm.org/out-of-band/1.1/invitation").is_some());
    let mediation = &index.protocols["https://didcomm.org/coordinate-mediation/1.0"];
    assert_eq!(mediation.messages.len(), 7, "{:?}", mediation.messages.iter().map(|m| &m.message_type).collect::<Vec<_>>());
    // Both spellings of trust ping, and the legacy prefix, find the same protocol.
    for requested in [
        "https://didcomm.org/trust_ping/1.0/ping",
        "https://didcomm.org/trust-ping/1.0",
        "did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/trust_ping/1.0/ping",
    ] {
        assert_eq!(index.resolve_protocol(requested).unwrap().piuri, "https://didcomm.org/trust_ping/1.0", "{requested}");
    }
    // DIDComm v2 protocols are untouched.
    assert_eq!(index.protocols["https://didcomm.org/coordinate-mediation/3.0"].source.name, "didcomm.org");
}

#[test]
fn didcomm_v1_and_the_attachment_formats_are_documents() {
    let index = index();
    let v1 = index.resolve_spec(Some("1.0")).unwrap();
    assert_eq!(v1.title, "DIDComm Messaging v1 (Aries RFCs)");
    assert_eq!(v1.didcomm_versions, ["^1.0"]);
    let ids: Vec<_> = v1.sections.iter().map(|s| s.id.as_str()).collect();
    for id in ["rfc0005", "rfc0008", "rfc0011", "rfc0017", "rfc0043", "rfc0092"] {
        assert!(ids.contains(&id), "{id} missing");
    }
    assert!(v1.sections.iter().any(|s| s.id.starts_with("rfc0043-") && s.markdown.contains("~l10n")));
    // The newest published spec is still the default.
    assert_eq!(index.resolve_spec(None).unwrap().version, "2.1");

    let formats = &index.documents["aries/attachment-formats"];
    assert_eq!(formats.didcomm_versions, ["^1.0"]);
    assert!(formats.versions["current"].sections.iter().any(|s| s.id == "rfc0592"));
}

#[test]
fn mismatched_folders_are_indexed_by_piuri() {
    let index = index();
    let pickup = &index.protocols["https://didcomm.org/message-pickup/4.0"];
    assert_eq!(pickup.source.path, "messagepickup/4.0/readme.md");
    assert!(index.protocols.contains_key("https://didcomm.org/questionanswer/1.0"));
}

#[test]
fn serves_every_spec_version() {
    let index = index();
    let spec = &index.documents["spec"];
    assert_eq!(spec.versions_newest_first(), ["2.1", "2.0", "1.0", "editors-draft"]);
    assert_eq!(index.resolve_spec(None).unwrap().version, "2.1");
    assert_eq!(spec.versions["2.0"].didcomm_versions, ["~2.0"]);
    for spec in spec.versions.values().filter(|v| v.version != "1.0") {
        assert!(spec.sections.iter().any(|s| s.id == "message-headers"), "{} lacks message-headers", spec.version);
        assert!(spec.sections.len() > 100);
    }
}

#[test]
fn core_protocols_have_their_schemas() {
    let index = index();
    for piuri in [
        "https://didcomm.org/discover-features/2.0",
        "https://didcomm.org/trust-ping/2.0",
        "https://didcomm.org/basicmessage/2.0",
        "https://didcomm.org/report-problem/2.0",
        "https://didcomm.org/coordinate-mediation/3.0",
        "https://didcomm.org/messagepickup/3.0",
        "https://didcomm.org/routing/2.0",
        "https://didcomm.org/out-of-band/2.0",
    ] {
        assert!(index.protocols[piuri].has_schemas(), "{piuri} has no schemas attached");
    }
    let mediation = &index.protocols["https://didcomm.org/coordinate-mediation/3.0"];
    assert_eq!(mediation.roles, ["mediator", "recipient"]);
    assert_eq!(mediation.messages.iter().filter(|m| m.schema().is_some()).count(), 7);
    assert_eq!(mediation.didcomm_versions(), ["^2.0"]);
}

/// Every schema compiles. Upstream examples are checked against them and mismatches
/// reported, not failed: examples legitimately use placeholders and skip required
/// headers (see schemas/README.md).
#[test]
fn schemas_compile_and_examples_are_reported() {
    let index = index();
    let (mut checked, mut mismatched) = (0, Vec::new());
    for doc in index.protocols.values() {
        for message in &doc.messages {
            for versioned in &message.schemas {
                let validator = jsonschema::validator_for(&versioned.schema)
                    .unwrap_or_else(|e| panic!("schema for {} doesn't compile: {e}", message.message_type));
                let examples = message.examples.iter().filter(|e| Envelope::of_message(e) == Some(versioned.envelope));
                for example in examples {
                    checked += 1;
                    if let Some(error) = validator.iter_errors(example).next() {
                        mismatched.push(format!("{} ({:?}): {error}", message.message_type, versioned.envelope));
                    }
                }
            }
        }
    }
    eprintln!("{checked} upstream examples checked against schemas; {} mismatched:", mismatched.len());
    for m in &mismatched {
        eprintln!("  {m}");
    }
    assert!(checked > 0);
}

/// `sources/revisions.toml` is what the container image (no `.git`) reports as each
/// source's revision, so it has to match the submodule commits this repository pins.
#[test]
fn the_revisions_file_matches_the_submodules() {
    let revisions: std::collections::HashMap<String, String> =
        toml::from_str(&std::fs::read_to_string("sources/revisions.toml").unwrap()).unwrap();
    for (name, path) in [
        ("didcomm.org", "sources/didcomm.org"),
        ("didcomm-messaging", "sources/didcomm-messaging"),
        ("aries-rfcs", "sources/aries-rfcs"),
        ("waci-didcomm", "sources/waci-didcomm"),
    ] {
        let output = std::process::Command::new("git").args(["ls-tree", "HEAD", path]).output().unwrap();
        let listing = String::from_utf8(output.stdout).unwrap();
        let pinned = listing.split_whitespace().nth(2).expect("a submodule entry");
        assert_eq!(
            revisions.get(name).map(String::as_str),
            Some(pinned),
            "sources/revisions.toml is stale for {name}; run scripts/update-sources.sh"
        );
    }
}

#[test]
fn the_spec_extensions_are_documents() {
    let index = index();
    let ids: Vec<_> = index.documents.keys().map(String::as_str).collect();
    for id in [
        "extension/advanced_sequencing",
        "extension/email_transport",
        "extension/filesystem_transport",
        "extension/l10n",
        "extension/libp2p_transport",
        "extension/return_route",
    ] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
    let l10n = &index.documents["extension/l10n"];
    assert_eq!(l10n.title, "DIDComm L10n Extension");
    assert_eq!(l10n.didcomm_versions, ["^2.0"]);
    let current = &l10n.versions["current"];
    assert!(current.sections.iter().any(|s| s.id == "scope" && s.markdown.contains("accept-lang")));
    assert_eq!(current.source.path, "extensions/l10n/main.md");
}

#[test]
fn waci_defines_the_v2_credential_protocols() {
    let index = index();
    for (piuri, messages) in [
        ("https://didcomm.org/issue-credential/3.0", ["propose-credential", "offer-credential", "request-credential", "issue-credential"]),
        ("https://didcomm.org/present-proof/3.0", ["propose-presentation", "request-presentation", "presentation", "presentation"]),
    ] {
        let doc = &index.protocols[piuri];
        assert_eq!(doc.source.name, "waci-didcomm", "{piuri} replaces didcomm.org's stub");
        assert_eq!(doc.didcomm_versions(), ["^2.0"]);
        assert_eq!(doc.status, "Proposed");
        for name in messages {
            assert!(doc.message(&format!("{piuri}/{name}")).is_some(), "{piuri}/{name}");
        }
    }
    let profile = &index.documents["waci-didcomm"];
    assert!(profile.versions["1.0"].sections.iter().any(|s| s.title == "Interoperability Profile"));
}

/// Shared definitions (decorators, attachments) are copied into each schema's $defs so
/// every file stands alone; this keeps the copies identical to schema-defs/.
#[test]
fn shared_schema_definitions_match_the_canonical_ones() {
    let canonical = |style: &str| -> serde_json::Map<String, serde_json::Value> {
        serde_json::from_str(&std::fs::read_to_string(format!("schema-defs/{style}.json")).unwrap()).unwrap()
    };
    let mut checked = 0;
    for style in ["v1", "v2"] {
        let shared = canonical(style);
        let mut stack = vec![std::path::PathBuf::from("schemas").join(style)];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let schema: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                for (name, definition) in schema["$defs"].as_object().into_iter().flatten() {
                    if let Some(expected) = shared.get(name) {
                        assert_eq!(definition, expected, "{}: $defs.{name} differs from schema-defs/{style}.json", path.display());
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 0);
}
