#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (C) 2026 plenarius
# SPDX-License-Identifier: GPL-3.0-or-later

# Runs a built nwn-bake on the made-up fixture (no game needed): every area of it in all five views, as WebP, with manifests.
# Usage: scripts/smoke-test.sh <path to the nwn-bake binary> [<empty folder for the fixture>]
set -euo pipefail
bin=$1
dir=${2:-${RUNNER_TEMP:-${TMPDIR:-/tmp}}/nwn-bake-smoke-$$}
cd "$(dirname "$0")/.."
cargo run --release --locked -q -p nwn-bake --example fixture -- "$dir" >/dev/null
"$bin" area-iso "$dir/fx.mod" fx_main fx_mark fx_flat fx_cave fx_step fx_chasm --out "$dir/out" --view all,top \
  --root "$dir/game" --user "$dir/user"
# Six areas: five pictures and a manifest each.
n=$(find "$dir/out" -type f | wc -l | tr -d ' ')
[ "$n" -eq 36 ] || { echo "expected 36 files in $dir/out, found $n" >&2; ls -R "$dir/out" >&2; exit 1; }
"$bin" area-scene "$dir/fx.mod" fx_main --root "$dir/game" --user "$dir/user" >/dev/null
echo "smoke test passed: $n files in $dir/out"
