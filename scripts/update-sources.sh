#!/bin/sh
# Move both documentation sources to their upstream default branches' latest commits,
# then re-run the index tests: a new folder/PIURI mismatch, or a schema that no longer
# attaches, fails them (fix with mappings/piuri-paths.toml or schemas/).
set -eu
cd "$(dirname "$0")/.."
git submodule update --init --remote --depth 1 sources/didcomm.org sources/didcomm-messaging
git submodule status
# Record the new commits for the container image, which has no .git to ask.
{
  echo "# The commit of each submodule in sources/, by source name -- for the container image,"
  echo "# which has no .git to ask (see index.revisions in config/default.toml). Kept in sync by"
  echo "# scripts/update-sources.sh; tests/index.rs fails if it disagrees with the submodules."
  echo "\"didcomm.org\" = \"$(git -C sources/didcomm.org rev-parse HEAD)\""
  echo "\"didcomm-messaging\" = \"$(git -C sources/didcomm-messaging rev-parse HEAD)\""
} > sources/revisions.toml
cargo test --test index
echo "Commit the new submodule commits if the tests pass: git add sources && git commit"
