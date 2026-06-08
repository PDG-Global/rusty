#!/usr/bin/env bash
# Deploy docs to Cloudflare Pages
# Project: rusty-docs (docs.rustycli.com)
#
# Prerequisites:
#   pip install mkdocs-material
#   npm install -g wrangler
#
# Usage:
#   ./scripts/deploy-docs.sh

set -euo pipefail

echo "Building docs..."
mkdocs build --clean --strict

echo "Deploying to Cloudflare Pages (rusty-docs)..."
wrangler pages deploy site-docs/ --project-name=rusty-docs --branch=main

echo "Deployed to https://docs.rustycli.com"
