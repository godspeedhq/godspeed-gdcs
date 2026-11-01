#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
#
# publish.sh - regenerates the documentation site from main and publishes
# it to GitHub Pages.
#
#   bash tools/docs/publish.sh
#
# Builds the pages locally first, so a broken generator stops here, then
# runs .github/workflows/pages.yml on main and waits for it. The first run
# switches the repository's Pages source to GitHub Actions. Needs python3
# and the gh CLI, signed in with admin rights on the repository.
#
# The site always reflects main as pushed: commit and merge first.

set -u

here=$(cd "$(dirname "$0")" && pwd)
slug=godspeedhq/godspeed-gdcs
python=$(command -v python3 || command -v python)
# Windows' python3 can be a shortcut to the Store, which runs nothing.
"$python" --version > /dev/null 2>&1 || python=python

command -v gh > /dev/null 2>&1 || { printf 'publish: the gh CLI is required\n' >&2; exit 1; }

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gdcs-docs.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
"$python" "$here/build.py" "$tmp/site" || { printf 'publish: the generator failed\n' >&2; exit 1; }

if ! gh api "repos/$slug/pages" > /dev/null 2>&1; then
    printf 'enabling GitHub Pages (source: GitHub Actions)\n'
    gh api -X POST "repos/$slug/pages" -f build_type=workflow > /dev/null || exit 1
fi

before=$(gh run list -R "$slug" --workflow pages.yml --limit 1 --json databaseId --jq '.[0].databaseId // 0')
gh workflow run pages.yml -R "$slug" --ref main || exit 1
# The new run appears after a moment.
run=$before
i=0
while [ "$run" = "$before" ] && [ "$i" -lt 30 ]; do
    sleep 2
    run=$(gh run list -R "$slug" --workflow pages.yml --limit 1 --json databaseId --jq '.[0].databaseId // 0')
    i=$((i + 1))
done
[ "$run" != "$before" ] || { printf 'publish: the workflow did not start\n' >&2; exit 1; }

gh run watch "$run" -R "$slug" --exit-status || { printf 'publish: the workflow failed: gh run view %s -R %s --log-failed\n' "$run" "$slug" >&2; exit 1; }
printf 'published: %s\n' "$(gh api "repos/$slug/pages" --jq .html_url)"
