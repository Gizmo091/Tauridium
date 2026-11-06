#!/bin/bash
# Compile src-tauri/icons/Tauridium.icon (Icon Composer) into src-tauri/icons/Assets.car,
# the file macOS 26+ uses to apply its icon treatment. Run after changing the .icon,
# then commit the result. Requires Xcode 26+.
#
# The .car is committed rather than built by `tauri build` because tauri-bundler's own
# actool call fails on Xcode 27: https://github.com/tauri-apps/tauri/issues/16386
set -euo pipefail

icons="$(cd "$(dirname "$0")/../src-tauri/icons" && pwd)"
out="$(mktemp -d)"
trap 'rm -rf "$out"' EXIT

compile() {
  actool "$icons/Tauridium.icon" --compile "$out" \
    --output-partial-info-plist "$out/partial.plist" \
    --app-icon Tauridium --include-all-app-icons \
    --enable-on-demand-resources NO --development-region en \
    --target-device mac --platform macosx --minimum-deployment-target 26.0 \
    </dev/null
}

# A previous actool run can leave a broken ibtoold daemon behind: restart it and retry once.
compile || { pkill -x ibtoold || true; sleep 1; compile; }

cp "$out/Assets.car" "$icons/Assets.car"
echo "Wrote $icons/Assets.car"
