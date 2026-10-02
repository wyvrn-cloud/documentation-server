#!/bin/sh
# Move both documentation sources to their upstream default branches' latest commits,
# then re-run the index tests: a new folder/PIURI mismatch, or a schema that no longer
# attaches, fails them (fix with mappings/piuri-paths.toml or schemas/).
set -eu
cd "$(dirname "$0")/.."
git submodule update --init --remote --depth 1 sources/didcomm.org sources/didcomm-messaging
git submodule status
cargo test --test index
echo "Commit the new submodule commits if the tests pass: git add sources && git commit"
