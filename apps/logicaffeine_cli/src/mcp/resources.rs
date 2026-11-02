//! MCP resources: the LOGOS learning corpus, carried inside the binary.
//!
//! A cargo-installed `largo` has no repo checkout on disk, so every resource travels with the
//! crate. Hand-written documents (the quickguide and the topical guides of `docs/`) and the
//! example corpus are embedded from the crate's agent pack, `assets/agent/`, which mirrors their
//! repository paths and is regenerated from them by `scripts/release/gen-agent-artifacts.sh`. Anything
//! derived from the language itself (the error catalog, in both shapes) is rendered live by
//! `logicaffeine_language::diagnostic`, so it cannot go stale. Agents load `logos://quickguide`
//! to learn the language in one read.

use std::borrow::Cow;

use logicaffeine_language::diagnostic::{catalog_json, catalog_markdown};
use serde_json::{json, Value};

use super::protocol::{self, INVALID_PARAMS};

/// Where a resource's text comes from.
enum Body {
    /// Embedded from `assets/agent/<source>`.
    Pack(&'static str),
    /// Rendered from the language crate on every read.
    Rendered(fn() -> String),
}

struct Resource {
    uri: &'static str,
    name: &'static str,
    /// The repository path this resource reproduces byte-for-byte.
    source: &'static str,
    body: Body,
}

impl Resource {
    fn text(&self) -> Cow<'static, str> {
        match self.body {
            Body::Pack(text) => Cow::Borrowed(text),
            Body::Rendered(render) => Cow::Owned(render()),
        }
    }
}

/// Every served markdown document.
const DOCS: &[Resource] = &[
    Resource {
        uri: "logos://quickguide",
        name: "LOGOS Quickguide — the whole language on one page",
        source: "LOGOS_QUICKGUIDE.md",
        body: Body::Pack(include_str!("../../assets/agent/LOGOS_QUICKGUIDE.md")),
    },
    Resource {
        uri: "logos://docs/getting-started",
        name: "Getting started — install, a first program and its test, a web page, English to logic",
        source: "docs/getting-started.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/getting-started.md")),
    },
    Resource {
        uri: "logos://docs/architecture",
        name: "Architecture — pipelines and execution tiers",
        source: "docs/architecture.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/architecture.md")),
    },
    Resource {
        uri: "logos://docs/cli",
        name: "CLI — every largo command",
        source: "docs/cli.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/cli.md")),
    },
    Resource {
        uri: "logos://docs/concurrency",
        name: "Concurrency — tasks, channels, CRDTs, networking",
        source: "docs/concurrency.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/concurrency.md")),
    },
    Resource {
        uri: "logos://docs/frontend",
        name: "Frontend — pages, components and the browser",
        source: "docs/frontend.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/frontend.md")),
    },
    Resource {
        uri: "logos://docs/lgx-reference",
        name: "LGX reference — every template tag, directive and modifier, generated from the compiler",
        source: "docs/lgx-reference.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/lgx-reference.md")),
    },
    Resource {
        uri: "logos://docs/lgx-stdlib",
        name: "LGX stdlib — forms, ui and session, the web standard library",
        source: "docs/lgx-stdlib.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/lgx-stdlib.md")),
    },
    Resource {
        uri: "logos://docs/web",
        name: "Web — the server half: routes, sessions, the database",
        source: "docs/web.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/web.md")),
    },
    Resource {
        uri: "logos://docs/execution-and-performance",
        name: "Execution & performance — tiers and benchmarks",
        source: "docs/execution-and-performance.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/execution-and-performance.md")),
    },
    Resource {
        uri: "logos://docs/imperative-mode",
        name: "Imperative mode — programs in English",
        source: "docs/imperative-mode.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/imperative-mode.md")),
    },
    Resource {
        uri: "logos://docs/logic-mode",
        name: "Logic mode — English to first-order logic",
        source: "docs/logic-mode.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/logic-mode.md")),
    },
    Resource {
        uri: "logos://docs/proof-and-verification",
        name: "Proof & verification — theorems and the kernel",
        source: "docs/proof-and-verification.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/proof-and-verification.md")),
    },
    Resource {
        uri: "logos://docs/studio-and-learn",
        name: "Studio & Learn — the web surfaces",
        source: "docs/studio-and-learn.md",
        body: Body::Pack(include_str!("../../assets/agent/docs/studio-and-learn.md")),
    },
    Resource {
        uri: "logos://docs/error-catalog",
        name: "Error catalog — every diagnostic code with its Socratic explanation",
        source: "docs/ERROR_CATALOG.md",
        body: Body::Rendered(catalog_markdown),
    },
];

/// Machine-shaped resources.
const DATA: &[Resource] = &[
    Resource {
        uri: "logos://errors",
        name: "Error catalog (JSON) — logos-errors/1",
        source: "apps/logicaffeine_web/public/errors.json",
        body: Body::Rendered(catalog_json),
    },
    Resource {
        uri: "logos://examples",
        name: "Example corpus (JSON) — logos-examples/1, runtime-verified expected outputs",
        source: "apps/logicaffeine_web/public/examples.json",
        body: Body::Pack(include_str!(
            "../../assets/agent/apps/logicaffeine_web/public/examples.json"
        )),
    },
];

/// The `resources/list` result.
pub fn list() -> Value {
    let mut resources: Vec<Value> = DOCS
        .iter()
        .map(|r| json!({"uri": r.uri, "name": r.name, "mimeType": "text/markdown"}))
        .collect();
    resources.extend(
        DATA.iter()
            .map(|r| json!({"uri": r.uri, "name": r.name, "mimeType": "application/json"})),
    );
    json!({"resources": resources})
}

/// The `resources/read` response line.
pub fn read(id: &Value, params: &Value) -> String {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return protocol::error(id, INVALID_PARAMS, "resources/read needs a 'uri'");
    };
    if let Some(r) = DOCS.iter().find(|r| r.uri == uri) {
        return protocol::ok(
            id,
            json!({"contents": [{"uri": r.uri, "mimeType": "text/markdown", "text": r.text()}]}),
        );
    }
    if let Some(r) = DATA.iter().find(|r| r.uri == uri) {
        return protocol::ok(
            id,
            json!({"contents": [{"uri": r.uri, "mimeType": "application/json", "text": r.text()}]}),
        );
    }
    protocol::error(id, INVALID_PARAMS, &format!("unknown resource '{uri}'"))
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/mcp/resources/tests.rs"]
mod tests;
