//! What a browser loads: a document, the page's WebAssembly module, and the host runtime.

use std::collections::BTreeMap;

use logicaffeine_compile::page_host::PAGE_STEM as STEM;
use logicaffeine_compile::vm::reach::HostEntries;

/// A built page: files by URL path, and the path the browser opens.
#[derive(Clone, Debug)]
pub struct PageBundle {
    files: BTreeMap<String, (String, Vec<u8>)>,
    entry: String,
}

impl PageBundle {
    /// A bundle holding one document, served as HTML at `entry`.
    pub fn new(entry: &str, document: impl Into<Vec<u8>>) -> PageBundle {
        let mut files = BTreeMap::new();
        files.insert(
            entry.to_string(),
            ("text/html; charset=utf-8".to_string(), document.into()),
        );
        PageBundle {
            files,
            entry: entry.to_string(),
        }
    }

    /// Add a file, typed by its extension through the table `largo serve` uses.
    pub fn with_file(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> PageBundle {
        self.files.insert(
            path.to_string(),
            (crate::content_type_for(path).to_string(), bytes.into()),
        );
        self
    }

    /// The document with `markup` placed at the end of its head, as `largo serve` places a
    /// project's stylesheet links.
    pub fn with_head_markup(mut self, markup: &str) -> PageBundle {
        if let Some((_, doc)) = self.files.get_mut(&self.entry) {
            let text =
                String::from_utf8_lossy(doc).replacen("</head>", &format!("{markup}</head>"), 1);
            *doc = text.into_bytes();
        }
        self
    }

    /// Serve the document at `path` instead, as a page declared at that path would be — and the
    /// module and host beside it, where the document's relative `./app.js` and the host's
    /// `./app.wasm` resolve from that path.
    pub fn at(mut self, path: &str) -> PageBundle {
        let dir = &path[..=path.rfind('/').unwrap_or(0)];
        for file in [format!("{STEM}.wasm"), format!("{STEM}.js")] {
            if let Some(bytes) = self.files.remove(&format!("/{file}")) {
                self.files.insert(format!("{dir}{file}"), bytes);
            }
        }
        if let Some(doc) = self.files.remove(&self.entry) {
            self.files.insert(path.to_string(), doc);
        }
        self.entry = path.to_string();
        self
    }

    /// A page as `largo emit wasm-dom` ships it: the module, the browser host, and the document
    /// shell with nothing pre-rendered — the first paint is the module's.
    pub fn page(source: &str) -> Result<PageBundle, String> {
        Self::page_with(source, &HostEntries::default())
    }

    /// [`PageBundle::page`] for a host that also mounts `entries`' components on demand: the
    /// module exports each one's mount, which `runtime.js`'s `__logosMount` calls. The document holds
    /// no pre-rendered markup, so the module draws fresh and carries no hydration.
    pub fn page_with(source: &str, entries: &HostEntries) -> Result<PageBundle, String> {
        let shell = logicaffeine_compile::web_host::browser_host_html(STEM);
        Self::with_module(source, shell, &entries.clone().drawing_fresh())
    }

    /// A page as `largo build` ships it: the same module and host, with the page's markup already
    /// in the document body, rendered by the build's own pre-render, and served at the URL its
    /// `## Page` declares, where the build puts it. A path with a `:segment` has no one URL to
    /// serve, so that page stays at `/`.
    pub fn prerendered(source: &str) -> Result<PageBundle, String> {
        if let Some(why) = crate::commands::web_bundle::prerender_refusal(source) {
            return Err(format!("the page cannot be pre-rendered: {why}"));
        }
        let doc = crate::prerender_page_html(source, STEM);
        let bundle = Self::with_module(source, doc, &HostEntries::default())?;
        Ok(match declared_page_path(source) {
            Some(path) if path != "/" && !path.contains(':') => bundle.at(&path),
            _ => bundle,
        })
    }

    fn with_module(
        source: &str,
        document: String,
        entries: &HostEntries,
    ) -> Result<PageBundle, String> {
        let wasm =
            logicaffeine_compile::compile::compile_to_wasm_with(source, entries).map_err(|e| {
                let d = e.diagnostic_parts(source);
                format!(
                    "the page does not compile to WebAssembly (line {}): {}",
                    d.line_num, d.message
                )
            })?;
        Ok(PageBundle::new("/", document)
            .with_file(&format!("/{STEM}.wasm"), wasm)
            .with_file(
                &format!("/{STEM}.js"),
                logicaffeine_compile::web_host::browser_host_js(STEM),
            ))
    }

    /// The path the browser opens.
    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// The bytes served at `path`, if the bundle holds it.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(|(_, b)| b.as_slice())
    }

    pub(crate) fn paths(&self) -> std::collections::HashSet<String> {
        self.files.keys().cloned().collect()
    }

    pub(crate) fn served(&self) -> BTreeMap<String, (String, Vec<u8>)> {
        self.files.clone()
    }
}

/// The URL the `## Page` in `source` declares, if the program auto-mounts one.
fn declared_page_path(source: &str) -> Option<String> {
    logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, interner| {
        parsed.ok().and_then(|(stmts, ..)| {
            logicaffeine_compile::page_host::auto_mounted_page_path(stmts, interner)
        })
    })
}
