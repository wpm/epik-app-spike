#!/bin/sh
# M3 demo: launch epik-app with the Epik persona and EpikMCP attached.
#
# Paths point at Bill's local checkout of epik-agent/Epik; the packaged app
# would resolve these itself (see FINDINGS.md, distribution).
#
# Read-only EpikMCP tools are pre-allowed so monitoring doesn't nag; mutating
# tools (issue_create, feature_launch, ...) still raise permission asks in
# the app, which is the point of the demo.

EPIK_CHECKOUT=${EPIK_CHECKOUT:-/Users/mcneill/Projects/Epik/Epik}
DIR=$(dirname "$0")

exec "$DIR/../target/debug/epik-app" \
  --model claude-sonnet-5 \
  --mcp-config "$DIR/epik-mcp.json" \
  --persona-file "$EPIK_CHECKOUT/plugin/skills/summon/persona.md" \
  --greet "Please introduce yourself." \
  --permission-mode default \
  --settings '{"permissions":{"allow":[
      "mcp__EpikMCP__issue_list","mcp__EpikMCP__issue_get",
      "mcp__EpikMCP__run_list","mcp__EpikMCP__run_get","mcp__EpikMCP__run_logs",
      "mcp__EpikMCP__feature_status","mcp__EpikMCP__repo_get",
      "mcp__EpikMCP__repo_default_branch",
      "mcp__EpikMCP__pr_list","mcp__EpikMCP__pr_get",
      "mcp__EpikMCP__issue_list_relationships","mcp__EpikMCP__project_list_items"
  ]}}'
