# Monthly restore drill (10 minutes)

A backup you have never restored is a hope, not a backup. Do this on the 1st of each month, on the
production VPS, without touching production data. Background and the full recovery procedure:
`docs/operations/backup-disaster-recovery.md`; entry point: `docs/operations/single-vps-deploy.md`.

`scripts/restore.sh` without `--prod` restores into a scratch database `embyr_restore_test` inside the
same Postgres container and drops it afterwards. The live `embyr` database is never touched.

## Steps

1. **Latest dump exists remotely and is fresh.**
   ```bash
   set -a; . ./backup.env; set +a
   rclone lsf --files-only "$BACKUP_REMOTE/daily" | sort | tail -n 3     # newest is < 36 h old
   rclone lsf --files-only "$BACKUP_REMOTE/weekly" | sort | tail -n 2
   ```
2. **Restore the newest remote dump into the scratch DB.**
   ```bash
   ./scripts/restore.sh --latest
   ```
   Expect: `restore finished; sanity numbers:` followed by `public tables`, `latest migration`,
   `accounts`, `users`, `projects`, then `DRILL OK`. Any error aborts (`--exit-on-error`).
3. **Compare the numbers to reality.** `accounts`/`users`/`projects` should match what you see in the
   console (give or take what happened since the dump). `latest migration` should equal the highest
   version in `migrations/` of the release you run. Write the three counts and the dump name in your log.
4. **Prove the encryption key is recoverable** (the part a database backup cannot prove): open your
   password manager entry for `EMBYR_ENCRYPTION_KEY (prod)`, and compare its SHA-256 with the live one
   without printing either:
   ```bash
   printf '%s' "$(grep -m1 '^EMBYR_ENCRYPTION_KEY=' .env.production | cut -d= -f2-)" | sha256sum
   # paste the value from the password manager into: printf '%s' "<paste>" | sha256sum   (same hash?)
   ```
5. **Optional, quarterly: a weekly dump.** `./scripts/restore.sh "$BACKUP_REMOTE/weekly/<name>"`.

## Do NOT start embyr-server against a restored copy

Its transaction sweeper reads the encrypted customer DSNs out of the database and reclaims/purges
transaction rows in the customers' real Postgres databases (ADR-054). A second server pointed at a
restored copy would do that from a stale copy of the state, next to the real one. The drill only proves
that the dump restores and contains data; the "does the app boot on it" proof is a real
`restore.sh --prod` during an incident (or on a brand-new VPS, as in the runbook's disaster section).

## If a step fails

- Step 1 shows nothing or stale files: backups are not running. `journalctl -u embyr-backup -e`,
  `systemctl list-timers embyr-backup.timer`, then `./scripts/backup.sh` by hand. Treat as an incident:
  you currently have no recent off-box backup.
- Step 2 errors in `pg_restore`: keep the failing dump (copy it aside), take a fresh one by hand with
  `./scripts/backup.sh`, and restore that. If fresh dumps also fail to restore, stop updates until fixed.
- Step 4 hashes differ: the offline copy is wrong or stale. Fix it today; this is the failure that
  turns a database restore into a data-loss event.
