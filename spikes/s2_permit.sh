#!/usr/bin/env bash
# S2: does a blocking PermissionRequest hook wait for an outside answer, and does Claude apply it?
# The hook writes the request to req.json, then waits for answer.txt (allow|deny) from a second process.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
run() {
  local answer=$1 delay=$2 dir; dir=$(mktemp -d); cd "$dir"
  cat >hook.sh <<'H'
#!/usr/bin/env bash
cat >"$PWD/req.json"
while [ ! -f "$PWD/answer.txt" ]; do sleep 0.2; done
printf '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"%s"}}}' "$(cat "$PWD/answer.txt")"
H
  chmod +x hook.sh
  printf '{"hooks":{"PermissionRequest":[{"hooks":[{"type":"command","command":"%s/hook.sh"}]}]}}' "$dir" >settings.json
  ( while [ ! -f req.json ]; do sleep 0.2; done; sleep "$delay"; echo "$answer" >answer.txt ) &
  local t0=$SECONDS
  claude -p --model haiku --settings settings.json \
    "Run exactly this bash command and nothing else: touch spike-ok. Then reply DONE or DENIED." </dev/null >out.txt 2>&1 || true
  wait
  local tool; tool=$(python3 -c 'import json; d=json.load(open("req.json")); print(d["tool_name"], d["tool_input"].get("command"))' 2>/dev/null || echo "no request")
  echo "answer=$answer waited=${delay}s total=$((SECONDS-t0))s request=[$tool] file=$([ -f spike-ok ] && echo created || echo absent) reply=$(tr -d '\n' <out.txt | cut -c1-60)"
}
run allow 8
run deny 3
