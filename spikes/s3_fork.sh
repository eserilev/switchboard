#!/usr/bin/env bash
# S3: does --resume <id> --fork-session leave the parent session file unchanged?
set -euo pipefail
dir=$(mktemp -d); cd "$dir"
json=$(claude -p --model haiku --output-format json "Remember the word: lantern. Reply with OK only.")
sid=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["session_id"])' <<<"$json")
file=$(find ~/.claude/projects -name "$sid.jsonl" | head -1)
before=$(sha256sum "$file" | cut -d' ' -f1); lines_before=$(wc -l <"$file")
f1=$(claude -p --model haiku --output-format json --resume "$sid" --fork-session "What word did I ask you to remember? One word.")
f2=$(claude -p --model haiku --output-format json --resume "$sid" --fork-session "Spell that word backwards. One word.")
after=$(sha256sum "$file" | cut -d' ' -f1)
for f in "$f1" "$f2"; do python3 -c 'import json,sys; d=json.load(sys.stdin); print("fork", d["session_id"], "->", d["result"].strip())' <<<"$f"; done
echo "parent $sid lines=$lines_before"
[ "$before" = "$after" ] && echo "PASS parent unchanged" || echo "FAIL parent changed: $(wc -l <"$file") lines"
f1id=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["session_id"])' <<<"$f1")
fu=$(claude -p --model haiku --output-format json --resume "$f1id" "And what was the word, uppercased? One word.")
python3 -c 'import json,sys; d=json.load(sys.stdin); print("follow-up", d["session_id"], "->", d["result"].strip())' <<<"$fu"
