#!/usr/bin/env bash
# Dispatch one Noren lane to its engine, with a quota gate and session reuse.
#
#   dispatch.sh <lane> <worktree> <prompt-file>
#
# A lane is one role bound to one engine and one persistent session. The quota
# gate runs first: an engine without headroom is skipped with exit 3 so the
# caller can hand the lane to its designated backup instead of burning a
# half-finished run. Session IDs persist under .fleet/sessions/ so a lane
# resumes its own conversation across days and quota resets.
set -uo pipefail

REPO="/Users/yoshinagatatsuya/Documents/apps/noren"
OPENCODE="$HOME/.opencode/bin/opencode"
FLEET_DIR="$REPO/.fleet"
SESSIONS="$FLEET_DIR/sessions"
LOGS="$FLEET_DIR/logs"
KALI_REPO="/home/matsulab/tatuya/apps/noren"

lane="${1:?lane required}"
worktree="${2:?worktree required}"
prompt_file="${3:?prompt file required}"

mkdir -p "$SESSIONS" "$LOGS"

# lane -> engine, quota-account, backup lane-engine
case "$lane" in
  glm-*)   engine=glm;   account=glm-main;   model="zai-coding-plan/glm-5.2" ;;
  qwen-*)  engine=qwen;  account=qwen-main;  model="qwencloud/qwen3.8-max-preview" ;;
  kimi-*)  engine=kimi;  account=kimi-main;  model="" ;;
  # Codex work goes through the codex-lab wrapper only (portal `codex-main`).
  # The plain `codex` binary maps to `codex-tatu`, which is nearly exhausted and
  # is deliberately not reachable from this script.
  lab-*)   engine=lab;   account=codex-main; model="" ;;
  *) echo "unknown lane: $lane" >&2; exit 2 ;;
esac

# --- quota gate -------------------------------------------------------------
if ! python3 "$REPO/scripts/fleet/quota.py" --gate "$account"; then
  echo "SKIP $lane: $account has no headroom" >&2
  exit 3
fi

sess_file="$SESSIONS/$lane"
log="$LOGS/$lane.$(date +%Y%m%d-%H%M%S).log"
prompt="$(cat "$prompt_file")"

echo "== lane=$lane engine=$engine worktree=$worktree log=$log"

case "$engine" in
  glm|qwen)
    if [ -s "$sess_file" ]; then
      sid="$(cat "$sess_file")"
      echo "-- resuming session $sid"
      "$OPENCODE" run --model "$model" --session "$sid" --auto \
        --dir "$worktree" --print-logs "$prompt" >"$log" 2>&1
      rc=$?
    else
      echo "-- new session"
      "$OPENCODE" run --model "$model" --auto --title "noren:$lane" \
        --dir "$worktree" --print-logs "$prompt" >"$log" 2>&1
      rc=$?
      # Capture the session id from the structured log for later resume.
      sid="$(grep -oE 'id=ses_[A-Za-z0-9]+' "$log" | head -1 | cut -d= -f2)"
      [ -n "$sid" ] && printf '%s' "$sid" >"$sess_file"
    fi
    ;;

  lab)
    # codex-lab keeps its own isolated CODEX_HOME via the wrapper.
    # workspace-write lets the lane write its own review file without
    # disabling the sandbox entirely.
    if [ -s "$sess_file" ]; then
      codex-lab exec resume "$(cat "$sess_file")" \
        --sandbox workspace-write --cd "$worktree" "$prompt" >"$log" 2>&1
      rc=$?
    else
      codex-lab exec --sandbox workspace-write --cd "$worktree" \
        "$prompt" >"$log" 2>&1
      rc=$?
      sid="$(grep -oiE 'session[ _-]?id[: ]+[0-9a-f-]{36}' "$log" \
             | head -1 | grep -oE '[0-9a-f-]{36}')"
      [ -n "$sid" ] && printf '%s' "$sid" >"$sess_file"
    fi
    ;;

  kimi)
    # Kimi is the only remote lane: it works the kali clone over SSH.
    #
    # Two delivery traps, both hit in practice:
    #  - a backgrounded dispatch has stdin on /dev/null, so ssh cannot forward
    #    a heredoc; the prompt must travel as a file (scp).
    #  - passing the prompt as `-p "$(cat file)"` through nested ssh quoting
    #    lets the prompt's own backticks and `$` be re-expanded by the remote
    #    shell, which silently produced an empty run. So a generated runner
    #    script reads the file itself and no substitution crosses the ssh
    #    boundary.
    remote_prompt="/tmp/noren-prompt-$lane.md"
    remote_runner="/tmp/noren-run-$lane.sh"
    if ! scp -q -o ConnectTimeout=10 "$prompt_file" "kali:$remote_prompt" \
         >"$log" 2>&1; then
      echo "scp of prompt failed" >>"$log"
      exit 1
    fi
    if [ -s "$sess_file" ]; then
      resume="--session $(cat "$sess_file")"
    else
      resume=""
    fi
    # The prompt is fed on stdin, never through `-p "$(cat ...)"`: prompts
    # contain markdown backticks, which command substitution would re-expand.
    {
      printf '#!/usr/bin/env bash\nset -uo pipefail\n'
      printf 'cd %s || exit 1\n' "$KALI_REPO"
      printf 'exec kimi-cli --print --final-message-only %s < %s\n' \
        "$resume" "$remote_prompt"
    } >"$FLEET_DIR/runner-$lane.sh"
    scp -q -o ConnectTimeout=10 "$FLEET_DIR/runner-$lane.sh" \
      "kali:$remote_runner" >>"$log" 2>&1
    ssh -n -o ConnectTimeout=10 kali \
      "bash -l $remote_runner" >>"$log" 2>&1
    rc=$?
    sid="$(grep -oE 'kimi -r [0-9a-f-]{36}' "$log" | head -1 | awk '{print $3}')"
    [ -n "$sid" ] && printf '%s' "$sid" >"$sess_file"
    ;;
esac

echo "-- lane=$lane rc=$rc"
tail -40 "$log"
exit $rc
