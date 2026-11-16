#!/usr/bin/env bash
# Migration policy lint (docs/operations/migration-policy.md).
#
# Fails when a migration file ADDED relative to the base (migrations/*.sql and
# migrations/customer/*.sql) contains a destructive / lock-heavy statement:
#   DROP TABLE, DROP COLUMN, RENAME, ALTER COLUMN ... TYPE, SET NOT NULL
# unless the file carries an explicit opt-out line with a reason:
#   -- migration-policy: allow <reason>
#
# Existing migrations are exempt: only files added vs the base are checked.
# SQL comments (`-- ...`) are ignored when matching, so prose can mention the keywords.
#
# Usage: scripts/check-migration-policy.sh [BASE]   (BASE: commit/ref, default origin/master)
set -euo pipefail

base="${1:-origin/master}"

# --no-renames: a renamed/copied migration counts as an added file.
files=$(git diff --name-only --no-renames --diff-filter=A "$base"...HEAD -- 'migrations/*.sql')
[ -z "$files" ] && { echo "migration-policy: no added migrations vs $base"; exit 0; }

pattern='(^|[^[:alnum:]_])(DROP[[:space:]]+TABLE|DROP[[:space:]]+COLUMN|RENAME|ALTER[[:space:]]+COLUMN[[:space:]].*[[:space:]]TYPE|SET[[:space:]]+NOT[[:space:]]+NULL)([^[:alnum:]_]|$)'
allow='^[[:space:]]*--[[:space:]]*migration-policy:[[:space:]]*allow[[:space:]]+[^[:space:]]'
fail=0

for f in $files; do
  if grep -qE "$allow" "$f"; then
    echo "migration-policy: $f opted out ($(grep -E "$allow" "$f" | head -1 | sed -E 's/^[[:space:]]*--[[:space:]]*//'))"
    continue
  fi
  hits=$(sed -E 's/--.*$//' "$f" | grep -inE "$pattern" || true)
  if [ -n "$hits" ]; then
    fail=1
    echo "::error file=$f::migration policy violation (see docs/operations/migration-policy.md)"
    echo "$hits" | sed "s|^|  $f:|"
  fi
done

if [ "$fail" -ne 0 ]; then
  echo
  echo "Use expand/contract instead, or add '-- migration-policy: allow <reason>' to the file."
  exit 1
fi
echo "migration-policy: OK ($(echo "$files" | wc -l | tr -d ' ') added migration(s) checked)"
