#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later

# Builds the nwn-bake command for Debian 12 (bookworm) in Docker, so it runs on Debian 12 and newer and on other Linux systems with
# glibc 2.36 or later: the glibc is the one system library the binary links against. Output: ./nwn-bake-debian12 (in the folder
# you run it from).
#
# Needs Docker. Cargo's downloads and the build are kept in two Docker volumes (nwn-bake-cargo, nwn-bake-target), so a second
# build is quicker.
set -euo pipefail
# The workspace: the folder above this script.
here=$(cd "$(dirname "$0")/.." && pwd)
# The commit, for the manifests' `renderer.commit` (git does not run inside the container).
commit=${NWN_BAKE_COMMIT:-$(git -C "$here" rev-parse --short=7 HEAD 2>/dev/null || true)}
docker run --rm -v "$here":/src -v nwn-bake-cargo:/usr/local/cargo/registry -v nwn-bake-target:/target -e CARGO_TARGET_DIR=/target \
  -e NWN_BAKE_COMMIT="$commit" -w /src rust:1-bookworm bash -c '
  apt-get update -qq && apt-get install -y -qq --no-install-recommends cmake clang libclang-dev pkg-config git ca-certificates >/dev/null &&
  cargo build --release --locked -p nwn-bake-cli &&
  cp /target/release/nwn-bake /src/nwn-bake-debian12 && chown $(stat -c %u:%g /src) /src/nwn-bake-debian12'
[ "$here" -ef . ] || mv "$here/nwn-bake-debian12" ./nwn-bake-debian12
ls -la ./nwn-bake-debian12
