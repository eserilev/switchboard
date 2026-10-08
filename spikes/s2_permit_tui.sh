#!/usr/bin/env bash
# S2b: the same hook in an interactive claude inside tmux. Does the dialog show while the hook waits?
set -uo pipefail
dir=$(mktemp -d); cd "$dir"
cat >hook.sh <<'H'
#!/usr/bin/env bash
cat >"$PWD/req.json"
while [ ! -f "$PWD/answer.txt" ]; do sleep 0.2; done
printf '{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"%s"}}}' "$(cat "$PWD/answer.txt")"
H
chmod +x hook.sh
printf '{"hooks":{"PermissionRequest":[{"hooks":[{"type":"command","command":"%s/hook.sh"}]}]}}' "$dir" >settings.json
tmux -L sb-s2 kill-server 2>/dev/null
tmux -L sb-s2 new-session -d -s t -x 160 -y 45 -c "$dir" "claude --model haiku --permission-mode default --settings settings.json"
sleep 5
tmux -L sb-s2 send-keys -t t Down Enter   # trust the new folder
sleep 6
tmux -L sb-s2 send-keys -t t "Run exactly this bash command and nothing else: touch spike-ok" Enter
for i in $(seq 1 60); do [ -f req.json ] && break; sleep 0.5; done
sleep 3
echo "=== screen while the hook waits (req.json $( [ -f req.json ] && echo present || echo missing)) ==="
tmux -L sb-s2 capture-pane -p -t t | grep -v '^\s*$' | tail -15
echo allow >answer.txt
sleep 8
echo "=== after allow: file $( [ -f spike-ok ] && echo created || echo absent) ==="
tmux -L sb-s2 capture-pane -p -t t | grep -v '^\s*$' | tail -6
tmux -L sb-s2 kill-server
