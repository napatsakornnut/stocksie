#!/bin/sh
# One-command deploy of the Dioxus app to Cloudflare Pages.
# Usage: dioxus/scripts/deploy-pages.sh
# Prereqs: ~/.cargo/bin/dx, `npx wrangler login` done once, secrets set once
# (see docs/DEPLOY_DIOXUS.md).
#
# Deploys from /tmp on purpose: /Users/ozone/.pnp.cjs (Yarn PnP) sits above
# every path under $HOME and breaks wrangler's esbuild bundling of the
# Functions bundle when the CWD is inside $HOME. Also stages _headers here
# because the dx build output has no static-asset pipeline.
set -e

CRATE_DIR="$(cd "$(dirname "$0")/.." && pwd)"
STAGE=/tmp/dx-pages
PROJECT=stocksie-dioxus
PUB="$HOME/.cargo/target/dx/stocksie-dioxus/release/web/public"

echo "==> dx build --web --release (from $CRATE_DIR)"
(cd "$CRATE_DIR" && RUSTC_WRAPPER="" "$HOME/.cargo/bin/dx" build --web --release)

echo "==> staging $PUB -> $STAGE (site + functions/ + _headers)"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp -R "$PUB/." "$STAGE/"
cp -R "$CRATE_DIR/functions" "$STAGE/functions"
cp -R "$CRATE_DIR/pwa/." "$STAGE/"
cat > "$STAGE/_headers" <<'EOF'
# hashed filenames -> cache forever; index.html keeps CF default (must-revalidate)
/assets/*
  Cache-Control: public, max-age=31536000, immutable
EOF

# PWA: inject manifest/theme-color/apple-touch + service-worker registration
# (idempotent — dx regenerates index.html on every build)
python3 - "$STAGE/index.html" <<'EOF'
import sys
path = sys.argv[1]
html = open(path, encoding="utf-8").read()
if 'rel="manifest"' not in html:
    head = ('<link rel="manifest" href="/manifest.webmanifest">'
            '<link rel="apple-touch-icon" href="/icon-180.png">'
            '<meta name="theme-color" content="#8250df">'
            '<meta name="apple-mobile-web-app-capable" content="yes">'
            '<meta name="apple-mobile-web-app-status-bar-style" content="default">')
    html = html.replace("<head>", "<head>" + head, 1)
if "serviceWorker" not in html:
    sw = ('<script>if("serviceWorker" in navigator){'
          'navigator.serviceWorker.register("/sw.js").catch(function(){})}</script>')
    html = html.replace("</body>", sw + "</body>", 1)
open(path, "w", encoding="utf-8").write(html)
print("PWA tags injected")
EOF

OLD_HASH=$(curl -s "https://$PROJECT.pages.dev/" | grep -o 'assets/[^"]*\.js' | head -1 || true)
NEW_HASH=$(grep -o 'assets/[^"]*\.js' "$STAGE/index.html" | head -1)

echo "==> deploying (old bundle: ${OLD_HASH:-none} / new: $NEW_HASH)"
cd "$STAGE"
npx wrangler pages deploy "$STAGE" --project-name "$PROJECT" --branch main --commit-dirty=true

echo ""
echo "==> verify:"
echo "    1. curl -s https://$PROJECT.pages.dev/ | grep -o 'assets/[^\"]*\\.js'  # must print $NEW_HASH"
echo "    2. curl -X POST https://$PROJECT.pages.dev/api/ai -H 'Content-Type: application/json' \\"
echo "         -d '{\"mode\":\"chat\",\"text\":\"ทดสอบ\",\"context\":{\"vaultBalanceSol\":\"0.001\",\"pendingRequestCount\":0,\"memberCount\":1,\"recentRequests\":[]}}'"
echo "    (see docs/DEPLOY_DIOXUS.md for the full checklist)"
