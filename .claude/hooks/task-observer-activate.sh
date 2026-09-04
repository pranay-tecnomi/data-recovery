#!/usr/bin/env bash
# SessionStart hook: enforced activation tier for the task-observer skill.
#
# The skill's references/environments.md describes three activation tiers and
# states that only the harness hook (tier 3) is enforced; frontmatter matching
# and a CLAUDE.md block are both probabilistic. This injects the activation
# instruction plus current log state into context on every session.
set -uo pipefail

LOG_DIR="/Users/pranay/Projects/Experiments/data-recovery/skill-observations/observation-log"

open_count=0
if [ -d "$LOG_DIR" ]; then
  open_count=$(grep -rl "status: OPEN" "$LOG_DIR" 2>/dev/null | wc -l | tr -d ' ')
fi

LOG_DIR="$LOG_DIR" OPEN_COUNT="$open_count" python3 -c '
import json, os
ctx = f"""Before the first tool call of this session - and before writing or proposing a
plan, not merely before executing one - invoke the task-observer skill AND
execute its Session Start Protocol (storage check, frontmatter scan, review
trigger). Loading the skill and running the protocol are separate steps; a
session that loads the file and stops has activated nothing. Any turn that
will involve a tool call counts; do not classify the session as "too simple"
from its opening message.

After completing each task, check the observation records written this session
and report a one-line summary (ids and titles, or "none logged and why").

The observation log for this project is pinned at:
  {os.environ["LOG_DIR"]}
Use that path; never resolve it from the current working directory.
Open observations currently in the log: {os.environ["OPEN_COUNT"]}"""
print(json.dumps({
    "hookSpecificOutput": {
        "hookEventName": "SessionStart",
        "additionalContext": ctx,
    },
    "suppressOutput": True,
}))
'
