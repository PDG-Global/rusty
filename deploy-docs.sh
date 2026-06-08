#!/usr/bin/env bash
set -euo pipefail

# Deploy Rusty docs to Cloudflare Pages
# Prerequisites: pip install -r docs/requirements.txt, wrangler authenticated

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "Building docs..."
mkdocs build --clean --site-dir site-docs

echo "Deploying to Cloudflare Pages (rusty-docs)..."
wrangler pages deploy site-docs/ --project-name rusty-docs

echo "Done. Docs will be available at https://docs.rustycli.com"
