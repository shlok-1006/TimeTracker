#!/usr/bin/env bash
#
# Pull the employee-details form into the directory tables, hourly.
#
# Google Forms has no webhook we can receive without Apps Script, so the HRMS route
# (RUH-HRMS: app/api/directory/sync-form) is a PULL and something has to call it. That
# something is this script, run from the VM's cron.
#
# It is safe to run as often as you like: TimeTracker records which form response produced
# each profile (migration 0043), and the sync writes a person only when their newest response
# is one it has not applied yet. A run with nothing new writes nothing at all.
#
# ── Install on the VM ────────────────────────────────────────────────────────────────────
#   sudo install -m 755 directory-sync.sh /usr/local/bin/directory-sync
#   sudo install -m 600 /dev/null /etc/timetracker/directory-sync.env   # then fill it in
#   printf '%s\n' \
#     'SHELL=/bin/bash' \
#     '17 * * * * root /usr/local/bin/directory-sync' \
#     | sudo tee /etc/cron.d/directory-sync
#
# Minute 17 rather than 0: nothing else needs to happen on the hour, and jobs that all fire
# at :00 are how a quiet box gets a noisy minute.
#
# ── /etc/timetracker/directory-sync.env ──────────────────────────────────────────────────
#   HRMS_URL=https://<the deployed HRMS host>
#   DIRECTORY_SYNC_TOKEN=<the same value set on the HRMS deployment; 24+ chars>
#
# The HRMS side additionally needs DIRECTORY_SYNC_TT_EMAIL / DIRECTORY_SYNC_TT_PASSWORD — a
# dedicated TimeTracker HR account. That account, not a person, is what the audit log will
# name as the author of every synced write, which is the honest attribution.

set -uo pipefail

ENV_FILE="${DIRECTORY_SYNC_ENV:-/etc/timetracker/directory-sync.env}"
LOG_FILE="${DIRECTORY_SYNC_LOG:-/var/log/directory-sync.log}"

# shellcheck source=/dev/null
[[ -r "$ENV_FILE" ]] && . "$ENV_FILE"

: "${HRMS_URL:?HRMS_URL is not set (see $ENV_FILE)}"
: "${DIRECTORY_SYNC_TOKEN:?DIRECTORY_SYNC_TOKEN is not set (see $ENV_FILE)}"

stamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }

# --max-time so a hung HRMS cannot leave overlapping runs stacked up by the next hour's cron.
body=$(curl -fsS --max-time 300 \
  -X POST "${HRMS_URL%/}/api/directory/sync-form" \
  -H "x-sync-token: ${DIRECTORY_SYNC_TOKEN}" \
  -H 'content-type: application/json' \
  -d '{"apply":true}' 2>&1)
rc=$?

if [[ $rc -ne 0 ]]; then
  # Printed, not just logged: cron mails whatever a job writes to stdout, and a sync that has
  # been failing silently for a fortnight is the failure mode worth being loud about.
  echo "$(stamp) directory-sync FAILED (curl rc=$rc): ${body}" | tee -a "$LOG_FILE"
  exit "$rc"
fi

# The route answers 200 with {"ok":false} for its own refusals (bad token, form unreachable),
# so HTTP success is not success.
if [[ "$body" != *'"ok":true'* ]]; then
  echo "$(stamp) directory-sync REFUSED: ${body}" | tee -a "$LOG_FILE"
  exit 1
fi

# The happy path is silent to cron and permanent in the log: most hours this reports only
# skips, and mailing that every hour would train everyone to filter the alerts away.
echo "$(stamp) ${body}" >> "$LOG_FILE"
