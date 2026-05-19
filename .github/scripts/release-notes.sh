#!/usr/bin/env bash
# Prints the Markdown body for a GitHub release to stdout: an
# auto-generated changelog from `git log` between the previous tag and
# the given tag, followed by fixed install/update instructions.
#
# Called from .github/workflows/release.yml. Requires `fetch-depth: 0`
# on the checkout step so the full tag history is available.
#
# Usage: release-notes.sh v0.4.3

set -euo pipefail

tag="${1:?usage: release-notes.sh <tag>}"

# `git describe HEAD^` from the tagged commit finds the most recent tag
# strictly before this one. On the very first tag it returns nothing and
# we fall back to "all history".
prev_tag=$(git describe --abbrev=0 --tags "${tag}^" 2>/dev/null || true)

if [ -n "$prev_tag" ]; then
    range="${prev_tag}..${tag}"
    range_label="seit ${prev_tag}"
else
    range="$tag"
    range_label="bisher"
fi

# Strip the release-bookkeeping commits — they would just add noise.
# Keep everything else as the user already shapes conventional-style
# subjects (feat:, fix:, docs:, refactor:, …) that read well as bullets.
changelog=$(
    git log "$range" --no-merges --pretty='format:- %s' \
        | grep -vE '^- (release|chore):' \
        || true
)

if [ -z "$changelog" ]; then
    changelog="- Kleinere Verbesserungen und Fehlerbehebungen."
fi

cat <<EOF
## Änderungen ${range_label}

${changelog}

## Installation

**macOS (Apple Silicon)**: DMG öffnen, App nach \`~/Applications\` ziehen.
Beim ersten Start Mikrofon- und Bedienungshilfen-Permissions erteilen.
Die App ist Apple-notarisiert — kein "Rechtsklick → Öffnen" mehr nötig,
und Permissions überleben Updates.

**Windows (x86_64, GPU-beschleunigt via Vulkan)**: NSIS-Installer
ausführen. Beim ersten Start zeigt Windows SmartScreen möglicherweise
eine Warnung — "Weitere Informationen → Trotzdem ausführen". GPU-
Beschleunigung funktioniert auf NVIDIA, AMD und Intel Arc/Xe out-of-the-
box (Vulkan-Treiber sind im GPU-Treiber enthalten).

Bestehende Installationen aktualisieren sich automatisch über den
eingebauten Updater (Tray → "Auf Updates prüfen…").
EOF
