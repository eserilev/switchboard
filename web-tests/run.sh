#!/usr/bin/env bash
# Runs the UI checks in headless Chromium. Prints each check and exits 1 on a failure.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
browser=$(command -v chromium || command -v google-chrome || command -v chromium-browser || true)
mac_chrome="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
if [ -z "$browser" ] && [ -x "$mac_chrome" ]; then browser=$mac_chrome; fi
if [ -z "$browser" ]; then echo "skip: no chromium"; exit 0; fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# The page body of index.html, without its own scripts; the harness adds the shim first.
python3 - "$here/../web/index.html" "$here/harness.html" "$work/page.html" "$here" <<'PY'
import re, sys
index, harness, out, here = sys.argv[1:]
body = open(index).read().split("<body>")[1].split("</body>")[0]
body = body.replace('src="vendor/', 'src="../web/vendor/').replace('src="app.js"', 'src="../web/app.js"').replace('src="review.js"', 'src="../web/review.js"')
page = open(harness).read().replace("@@BODY@@", body)
open(out, "w").write(page.replace('src="shim.js"', f'src="{here}/shim.js"').replace('src="checks.js"', f'src="{here}/checks.js"').replace('"../web/', f'"{here}/../web/'))
PY
"$browser" --headless --disable-gpu --no-sandbox --allow-file-access-from-files --virtual-time-budget=15000 --dump-dom "file://$work/page.html" 2>/dev/null >"$work/dom.html"
python3 - "$work/dom.html" <<'PY'
import html, json, re, sys
dom = open(sys.argv[1]).read()
m = re.search(r'<pre id="__results">(.*?)</pre>', dom, re.S)
if not m:
    print("FAIL: no results in the page"); sys.exit(1)
results = json.loads(html.unescape(m.group(1)))
bad = [r for r in results if not r["ok"]]
for r in results:
    print(("ok   " if r["ok"] else "FAIL ") + r["name"] + ("" if r["ok"] else f"  -> {r.get('info')}"))
print(f"{len(results) - len(bad)}/{len(results)} passed")
sys.exit(1 if bad else 0)
PY
