#!/usr/bin/env sh
# Gives every crate the version VERSION, as a release's tag names it: `./scripts/set-version.sh 1.2.3`.
# The version in the repository is only what a build from source says of itself.
set -eu
cd "$(dirname "$0")/.."
version=${1:?usage: set-version.sh VERSION}
# The first `version` of the root manifest is the workspace's, which every crate takes.
perl -pi -e 'if (!$done && s/^version = ".*"/version = "'"$version"'"/) { $done = 1 }' Cargo.toml
grep -q "^version = \"$version\"\$" Cargo.toml
