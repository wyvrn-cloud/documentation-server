//! The index over the real sources (the submodules) with the repository's own
//! `config/default.toml`. Needs `git submodule update --init`.

use documentation_server::{config::Config, index::Index};

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
    assert_eq!(index.protocols.len(), readmes);
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
    let versions: Vec<_> = index.specs.keys().map(String::as_str).collect();
    assert_eq!(versions, ["2.0", "2.1", "editors-draft"]);
    assert_eq!(index.resolve_spec(None).unwrap().version, "2.1");
    for spec in index.specs.values() {
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
    assert_eq!(mediation.messages.iter().filter(|m| m.schema.is_some()).count(), 7);
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
            let Some(schema) = &message.schema else { continue };
            let validator = jsonschema::validator_for(schema)
                .unwrap_or_else(|e| panic!("schema for {} doesn't compile: {e}", message.message_type));
            for example in message.examples.iter().filter(|e| e.get("type").is_some()) {
                checked += 1;
                if let Some(error) = validator.iter_errors(example).next() {
                    mismatched.push(format!("{}: {error}", message.message_type));
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
    for (name, path) in [("didcomm.org", "sources/didcomm.org"), ("didcomm-messaging", "sources/didcomm-messaging")] {
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
