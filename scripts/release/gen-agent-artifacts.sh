#!/usr/bin/env bash
# Regenerate every agent-facing generated artifact:
#
#   docs/lgx-reference.md, docs/lgx-stdlib.md      (the generated LGX guides, via lgx/gen-reference.sh)
#   docs/ERROR_CATALOG.md                          (error catalog, markdown)
#   apps/logicaffeine_web/public/errors.json       (error catalog, machine shape)
#   apps/logicaffeine_web/public/llms-full.txt     (the one-file curriculum)
#   apps/logicaffeine_web/public/examples.json     (the example corpus)
#   apps/logicaffeine_web/public/agent.txt         (the agent guide the front page's prompt names)
#   apps/logicaffeine_cli/assets/agent/            (largo's MCP agent pack: a mirror of the
#                                                  quickguide, the docs/ guides and examples.json)
#
# Each is byte-locked by its test file; this script runs the #[ignore]d
# regenerators in dependency order (the LGX guides and the catalog feed
# llms-full.txt and the agent pack; the web exports feed the agent pack), then
# re-runs the locks to prove the checked-in files match.
set -euo pipefail
cd "$(dirname "$0")/../.."

./scripts/release/lgx/gen-reference.sh
./scripts/release/gen-error-catalog.sh

LOGOS_REGEN=1 cargo test -p logicaffeine-tests --test llms_lock \
    regenerate_llms_full -- --ignored --exact
LOGOS_REGEN=1 cargo test -p logicaffeine-tests --features web-tests --test examples_export \
    regenerate_examples_json -- --ignored --exact
LOGOS_REGEN=1 cargo test -p logicaffeine-tests --features web-tests --test landing_page \
    regenerate_agent_txt -- --ignored --exact
LOGOS_REGEN=1 cargo test -p logicaffeine-cli --lib \
    mcp::resources::tests::regenerate_agent_pack -- --ignored --exact

cargo test -p logicaffeine-tests --features web-tests --test llms_lock --test examples_export
cargo test -p logicaffeine-tests --features web-tests --test landing_page the_served_agent_guide_is_the_instructions
cargo test -p logicaffeine-cli --lib mcp::resources

echo "agent artifacts regenerated and locks green"
