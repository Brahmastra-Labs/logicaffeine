use std::sync::Arc;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::document::DocumentState;
use crate::flycheck::{CargoFlycheck, Flycheck, FlycheckRunner};
use crate::scheduler::{Scheduler, DEBOUNCE};
use crate::semantic_tokens;
use crate::state::ServerState;
use crate::workspace::WorkspaceIndex;

pub struct LogicAffeineServer {
    client: Client,
    state: Arc<ServerState>,
    scheduler: Arc<Scheduler>,
    workspace_index: Arc<WorkspaceIndex>,
    workspace_roots: std::sync::Mutex<Vec<std::path::PathBuf>>,
    flycheck: Arc<Flycheck>,
    /// `logicaffeine.flycheck.enable` — on-save rustc analysis (default on).
    flycheck_enabled: std::sync::atomic::AtomicBool,
}

impl LogicAffeineServer {
    pub fn new(client: Client) -> Self {
        Self::with_flycheck(client, Box::new(CargoFlycheck::new()))
    }

    /// Construct with an injected check engine — the seam mock-runner tests
    /// use to prove merge/staleness/dedup behavior without a toolchain.
    pub fn with_flycheck(client: Client, runner: Box<dyn FlycheckRunner>) -> Self {
        LogicAffeineServer {
            client,
            state: Arc::new(ServerState::new()),
            scheduler: Arc::new(Scheduler::new()),
            workspace_index: Arc::new(WorkspaceIndex::new()),
            workspace_roots: std::sync::Mutex::new(Vec::new()),
            flycheck: Arc::new(Flycheck::new(runner)),
            flycheck_enabled: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// The cache key for this document's flycheck runs: the workspace root
    /// when one is open, else the file's own directory.
    fn workspace_key(&self, uri: &Url) -> String {
        if let Some(root) = self.workspace_roots.lock().unwrap().first() {
            return root.display().to_string();
        }
        uri.to_file_path()
            .ok()
            .and_then(|p| p.parent().map(|d| d.display().to_string()))
            .unwrap_or_else(|| uri.to_string())
    }

    /// Analyze `text` off the async runtime, and — if `generation` is still
    /// current when done — install the snapshot and publish its diagnostics.
    async fn analyze_and_publish(
        client: Client,
        state: Arc<ServerState>,
        scheduler: Arc<Scheduler>,
        flycheck: Arc<Flycheck>,
        uri: Url,
        text: String,
        version: i32,
        generation: u64,
    ) {
        let analysis_uri = uri.clone();
        let document = match tokio::task::spawn_blocking(move || {
            DocumentState::with_uri(text, version, Some(&analysis_uri))
        })
        .await
        {
            Ok(document) => document,
            Err(join_error) => {
                // Defense in depth: the parser is total by construction, but
                // an analysis panic must degrade to "stale snapshot", never
                // take the document (or the server) down.
                log::error!("analysis panicked for {uri}: {join_error}");
                return;
            }
        };

        if !scheduler.is_current(&uri, generation) {
            return;
        }
        let snapshot = state.install_snapshot(uri.clone(), document);
        let mut diagnostics = snapshot.diagnostics.clone();
        diagnostics.extend(flycheck.diagnostics_for(&uri));
        client
            .publish_diagnostics(uri, diagnostics, Some(snapshot.version))
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for LogicAffeineServer {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        let mut roots = Vec::new();
        if let Some(folders) = params.workspace_folders {
            roots.extend(folders.iter().filter_map(|f| f.uri.to_file_path().ok()));
        }
        #[allow(deprecated)]
        if roots.is_empty() {
            if let Some(root) = params.root_uri.and_then(|u| u.to_file_path().ok()) {
                roots.push(root);
            }
        }
        *self.workspace_roots.lock().unwrap() = roots;

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::INCREMENTAL),
                        save: Some(TextDocumentSyncSaveOptions::SaveOptions(SaveOptions {
                            include_text: Some(false),
                        })),
                        ..Default::default()
                    },
                )),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: semantic_tokens::legend(),
                            full: Some(SemanticTokensFullOptions::Delta { delta: Some(true) }),
                            range: Some(true),
                            ..Default::default()
                        },
                    ),
                ),
                document_symbol_provider: Some(OneOf::Left(true)),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        ".".to_string(),
                        ":".to_string(),
                        "'".to_string(),
                    ]),
                    ..Default::default()
                }),
                references_provider: Some(OneOf::Left(true)),
                signature_help_provider: Some(SignatureHelpOptions {
                    trigger_characters: Some(vec![
                        " ".to_string(),
                        ",".to_string(),
                    ]),
                    ..Default::default()
                }),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                folding_range_provider: Some(FoldingRangeProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Left(true)),
                code_lens_provider: Some(CodeLensOptions {
                    resolve_provider: Some(false),
                }),
                document_formatting_provider: Some(OneOf::Left(true)),
                document_range_formatting_provider: Some(OneOf::Left(true)),
                document_on_type_formatting_provider: Some(DocumentOnTypeFormattingOptions {
                    first_trigger_character: ".".to_string(),
                    more_trigger_character: None,
                }),
                document_highlight_provider: Some(OneOf::Left(true)),
                selection_range_provider: Some(SelectionRangeProviderCapability::Simple(true)),
                call_hierarchy_provider: Some(CallHierarchyServerCapability::Simple(true)),
                diagnostic_provider: Some(DiagnosticServerCapabilities::Options(
                    DiagnosticOptions {
                        identifier: None,
                        inter_file_dependencies: false,
                        workspace_diagnostics: false,
                        work_done_progress_options: Default::default(),
                    },
                )),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "logicaffeine-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        log::info!("LogicAffeine LSP initialized");
        // Index the workspace in the background; requests answer from
        // whatever has landed so far.
        let roots = self.workspace_roots.lock().unwrap().clone();
        let index = Arc::clone(&self.workspace_index);
        tokio::task::spawn_blocking(move || {
            for root in roots {
                index.scan_folder(&root);
            }
        });
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        // `logicaffeine.flycheck.enable` — the only server-side setting today.
        let enabled = params
            .settings
            .pointer("/logicaffeine/flycheck/enable")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let was = self
            .flycheck_enabled
            .swap(enabled, std::sync::atomic::Ordering::SeqCst);
        if was && !enabled {
            // Turning it off retracts published findings: republish every
            // open document's interactive-only set.
            let uris = self.scheduler.open_uris();
            for uri in uris {
                self.flycheck.forget(&uri);
                if let Some(doc) = self.state.snapshot(&uri) {
                    self.client
                        .publish_diagnostics(uri, doc.diagnostics.clone(), Some(doc.version))
                        .await;
                }
            }
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        if !self
            .flycheck_enabled
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            // Workspace re-index still happens; only the rustc pass is off.
            if let Ok(path) = params.text_document.uri.to_file_path() {
                let index = Arc::clone(&self.workspace_index);
                tokio::task::spawn_blocking(move || index.index_file(&path));
            }
            return;
        }
        let uri = params.text_document.uri;
        if let Ok(path) = uri.to_file_path() {
            let index = Arc::clone(&self.workspace_index);
            tokio::task::spawn_blocking(move || index.index_file(&path));
        }

        // Flycheck: run rustc's analysis over the saved text in the
        // background; the generation guard means a newer save always wins.
        let Some((text, version)) = self.scheduler.current_text(&uri) else {
            return;
        };
        let generation = self.flycheck.begin_save(&uri);
        let workspace_key = self.workspace_key(&uri);
        let client = self.client.clone();
        let state = Arc::clone(&self.state);
        let flycheck = Arc::clone(&self.flycheck);
        tokio::spawn(async move {
            let run_text = text.clone();
            let run_flycheck = Arc::clone(&flycheck);
            let findings = tokio::task::spawn_blocking(move || {
                run_flycheck.run(&run_text, &workspace_key)
            })
            .await
            .expect("flycheck task panicked");

            // Unavailable toolchain: interactive-only diagnostics, silently.
            let Some(findings) = findings else { return };

            let interactive = state
                .snapshot(&uri)
                .map(|doc| doc.diagnostics.clone())
                .unwrap_or_default();
            let Some(rustc_diagnostics) =
                flycheck.complete(&uri, generation, findings, &text, &interactive)
            else {
                return; // a newer save or an edit superseded this run
            };

            let mut diagnostics = interactive;
            diagnostics.extend(rustc_diagnostics);
            client
                .publish_diagnostics(uri, diagnostics, Some(version))
                .await;
        });
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        let index = Arc::clone(&self.workspace_index);
        tokio::task::spawn_blocking(move || {
            for change in params.changes {
                match change.typ {
                    FileChangeType::DELETED => index.remove(&change.uri),
                    _ => {
                        if let Ok(path) = change.uri.to_file_path() {
                            index.index_file(&path);
                        }
                    }
                }
            }
        });
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Option<Vec<SymbolInformation>>> {
        let hits = self.workspace_index.query(&params.query, 256);
        if hits.is_empty() {
            return Ok(None);
        }
        #[allow(deprecated)]
        Ok(Some(
            hits.into_iter()
                .map(|s| SymbolInformation {
                    name: s.name,
                    kind: s.kind,
                    tags: None,
                    deprecated: None,
                    location: s.location,
                    container_name: s.container,
                })
                .collect(),
        ))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        let version = params.text_document.version;

        let generation = self.scheduler.open(uri.clone(), text.clone(), version);
        // First analysis runs immediately — no debounce on open.
        Self::analyze_and_publish(
            self.client.clone(),
            Arc::clone(&self.state),
            Arc::clone(&self.scheduler),
            Arc::clone(&self.flycheck),
            uri,
            text,
            version,
            generation,
        )
        .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let version = params.text_document.version;

        let Some(generation) =
            self.scheduler
                .apply_changes(&uri, params.content_changes, version)
        else {
            return;
        };

        // Edits invalidate flycheck findings — their positions would lie.
        self.flycheck.clear(&uri);

        // Debounce: analysis fires only if no newer edit arrives in the
        // window; the generation guard drops stale results after it.
        let client = self.client.clone();
        let state = Arc::clone(&self.state);
        let scheduler = Arc::clone(&self.scheduler);
        let flycheck = Arc::clone(&self.flycheck);
        tokio::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            let Some((text, version)) = scheduler.current_if(&uri, generation) else {
                return;
            };
            Self::analyze_and_publish(
                client, state, scheduler, flycheck, uri, text, version, generation,
            )
            .await;
        });
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.scheduler.close(&params.text_document.uri);
        self.state.close_document(&params.text_document.uri);
        self.flycheck.forget(&params.text_document.uri);
        // Clear diagnostics on close
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let data = semantic_tokens::encode_document_tokens(&doc);
        let result_id = self.state.cache_semantic_tokens(uri.clone(), data.clone());

        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: Some(result_id),
            data,
        })))
    }

    async fn semantic_tokens_full_delta(
        &self,
        params: SemanticTokensDeltaParams,
    ) -> Result<Option<SemanticTokensFullDeltaResult>> {
        let uri = &params.text_document.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let data = semantic_tokens::encode_document_tokens(&doc);
        let previous = self.state.cached_semantic_tokens(uri);
        let result_id = self.state.cache_semantic_tokens(uri.clone(), data.clone());

        match previous {
            Some((prev_id, prev_data)) if prev_id == params.previous_result_id => {
                let edits = semantic_tokens::semantic_token_edits(&prev_data, &data);
                Ok(Some(SemanticTokensFullDeltaResult::TokensDelta(
                    SemanticTokensDelta {
                        result_id: Some(result_id),
                        edits,
                    },
                )))
            }
            // Unknown or stale id: answer with the full set.
            _ => Ok(Some(SemanticTokensFullDeltaResult::Tokens(SemanticTokens {
                result_id: Some(result_id),
                data,
            }))),
        }
    }

    async fn semantic_tokens_range(
        &self,
        params: SemanticTokensRangeParams,
    ) -> Result<Option<SemanticTokensRangeResult>> {
        let uri = &params.text_document.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let start = doc.line_index.offset(params.range.start);
        let end = doc.line_index.offset(params.range.end);
        let data = semantic_tokens::encode_document_tokens_in_range(&doc, start, end);

        Ok(Some(SemanticTokensRangeResult::Tokens(SemanticTokens {
            result_id: None,
            data,
        })))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let symbols = crate::document_symbols::document_symbols(&doc);
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        if let Some(local) = crate::definition::goto_definition(&doc, position, uri) {
            return Ok(Some(local));
        }

        // Cross-file: resolve the name under the cursor against the
        // workspace index when this document doesn't define it.
        let offset = doc.line_index.offset(position);
        let name = doc
            .tokens
            .iter()
            .find(|t| t.span.start <= offset && offset < t.span.end)
            .and_then(|t| crate::index::resolve_token_name(t, &doc.interner))
            .map(|n| n.to_string());
        Ok(name
            .and_then(|n| self.workspace_index.definition_of(&n))
            .map(GotoDefinitionResponse::Scalar))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::hover::hover(&doc, position))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::completion::completions(&doc, position))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let include_declaration = params.context.include_declaration;
        let mut locations =
            crate::references::find_references(&doc, position, uri, include_declaration);

        // Cross-file: LIVE buffers answer for themselves; the workspace
        // index answers for everything on disk that isn't open.
        if let Some(name) = crate::references::name_at(&doc, position) {
            if crate::references::is_cross_file_symbol(&doc, &name) {
                let open_docs = self.state.open_documents();
                for (other_uri, other_doc) in &open_docs {
                    if other_uri == uri {
                        continue;
                    }
                    for range in crate::references::cross_file_candidates(
                        other_doc,
                        &name,
                        include_declaration,
                    ) {
                        locations.push(Location { uri: other_uri.clone(), range });
                    }
                }
                let skip: Vec<&Url> = open_docs.iter().map(|(u, _)| u).collect();
                locations.extend(self.workspace_index.references_of(&name, &skip));
            }
        }

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::signature_help::signature_help(&doc, position))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let range = params.range;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let actions = crate::code_actions::code_actions(&doc, range, uri);
        if actions.is_empty() {
            Ok(None)
        } else {
            Ok(Some(actions))
        }
    }

    async fn rename(&self, params: RenameParams) -> Result<Option<WorkspaceEdit>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let new_name = params.new_name;
        let mut edit = match crate::rename::rename(&doc, position, new_name.clone(), uri) {
            Some(edit) => edit,
            None => return Ok(None),
        };

        // Cross-file: LIVE buffers answer for themselves; the workspace
        // index answers for everything on disk that isn't open.
        if let Some(name) = crate::references::name_at(&doc, position) {
            if crate::references::is_cross_file_symbol(&doc, &name) {
                let changes = edit.changes.get_or_insert_with(Default::default);
                let open_docs = self.state.open_documents();
                for (other_uri, other_doc) in &open_docs {
                    if other_uri == uri {
                        continue;
                    }
                    let ranges =
                        crate::references::cross_file_candidates(other_doc, &name, true);
                    if !ranges.is_empty() {
                        changes.entry(other_uri.clone()).or_default().extend(
                            ranges.into_iter().map(|range| TextEdit {
                                range,
                                new_text: new_name.clone(),
                            }),
                        );
                    }
                }
                let skip: Vec<&Url> = open_docs.iter().map(|(u, _)| u).collect();
                for location in self.workspace_index.references_of(&name, &skip) {
                    changes.entry(location.uri).or_default().push(TextEdit {
                        range: location.range,
                        new_text: new_name.clone(),
                    });
                }
            }
        }

        Ok(Some(edit))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let position = params.position;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::rename::prepare_rename(&doc, position).map(|(range, text)| {
            PrepareRenameResponse::RangeWithPlaceholder {
                range,
                placeholder: text,
            }
        }))
    }

    async fn folding_range(&self, params: FoldingRangeParams) -> Result<Option<Vec<FoldingRange>>> {
        let uri = &params.text_document.uri;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let ranges = crate::folding::folding_ranges(&doc);
        if ranges.is_empty() {
            Ok(None)
        } else {
            Ok(Some(ranges))
        }
    }

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let uri = &params.text_document.uri;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let hints = crate::inlay_hints::inlay_hints(&doc, params.range);
        if hints.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hints))
        }
    }

    async fn code_lens(&self, params: CodeLensParams) -> Result<Option<Vec<CodeLens>>> {
        let uri = &params.text_document.uri;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let lenses = crate::code_lens::code_lenses(&doc, uri);
        if lenses.is_empty() {
            Ok(None)
        } else {
            Ok(Some(lenses))
        }
    }

    async fn document_highlight(
        &self,
        params: DocumentHighlightParams,
    ) -> Result<Option<Vec<DocumentHighlight>>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };
        Ok(crate::document_highlights::document_highlights(
            &doc,
            params.text_document_position_params.position,
        ))
    }

    async fn selection_range(
        &self,
        params: SelectionRangeParams,
    ) -> Result<Option<Vec<SelectionRange>>> {
        let doc = match self.state.snapshot(&params.text_document.uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };
        Ok(Some(crate::selection_ranges::selection_ranges(
            &doc,
            &params.positions,
        )))
    }

    async fn on_type_formatting(
        &self,
        params: DocumentOnTypeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document_position.text_document.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        // Normalize the line the sentence just closed on — the same rule set
        // `largo fmt` applies, one sentence at a time.
        let line_number = params.text_document_position.position.line as usize;
        let Some(line) = doc.source.split('\n').nth(line_number) else {
            return Ok(None);
        };
        // Inside a multiline string every byte is content: an odd number of
        // """ delimiters before this line means the sentence-closing `.`
        // was typed inside one — never touch it. The LIVE text decides (the
        // snapshot can lag a keystroke behind).
        let live = self
            .scheduler
            .current_text(uri)
            .map(|(text, _)| text)
            .unwrap_or_else(|| doc.source.clone());
        let line_start_offset: usize = live
            .split('\n')
            .take(line_number)
            .map(|l| l.len() + 1)
            .sum();
        let delimiters_before = live[..line_start_offset.min(live.len())]
            .matches("\"\"\"")
            .count();
        if delimiters_before % 2 == 1 {
            return Ok(None);
        }
        let formatted = logicaffeine_language::source_format::format_line(line);
        if formatted == line {
            return Ok(None);
        }
        let end_character = line.encode_utf16().count() as u32;
        Ok(Some(vec![TextEdit {
            range: Range {
                start: Position { line: line_number as u32, character: 0 },
                end: Position { line: line_number as u32, character: end_character },
            },
            new_text: formatted,
        }]))
    }

    async fn prepare_call_hierarchy(
        &self,
        params: CallHierarchyPrepareParams,
    ) -> Result<Option<Vec<CallHierarchyItem>>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };
        Ok(crate::call_hierarchy::prepare(
            &doc,
            params.text_document_position_params.position,
            uri,
        ))
    }

    async fn incoming_calls(
        &self,
        params: CallHierarchyIncomingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
        let uri = &params.item.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };
        Ok(Some(crate::call_hierarchy::incoming_calls(
            &doc,
            &params.item,
            uri,
        )))
    }

    async fn outgoing_calls(
        &self,
        params: CallHierarchyOutgoingCallsParams,
    ) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
        let uri = &params.item.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };
        Ok(Some(crate::call_hierarchy::outgoing_calls(
            &doc,
            &params.item,
            uri,
        )))
    }

    async fn diagnostic(
        &self,
        params: DocumentDiagnosticParams,
    ) -> Result<DocumentDiagnosticReportResult> {
        let uri = &params.text_document.uri;
        let (items, result_id) = match self.state.snapshot(uri) {
            Some(doc) => {
                let mut items = doc.diagnostics.clone();
                let rustc = self.flycheck.diagnostics_for(uri);
                // The id covers BOTH engines: a flycheck completion changes
                // the report without a version bump.
                let result_id = format!("{}:{}", doc.version, rustc.len());
                items.extend(rustc);
                (items, result_id)
            }
            None => (Vec::new(), "closed".to_string()),
        };

        if params.previous_result_id.as_deref() == Some(result_id.as_str()) {
            return Ok(DocumentDiagnosticReportResult::Report(
                DocumentDiagnosticReport::Unchanged(RelatedUnchangedDocumentDiagnosticReport {
                    related_documents: None,
                    unchanged_document_diagnostic_report: UnchangedDocumentDiagnosticReport {
                        result_id,
                    },
                }),
            ));
        }

        Ok(DocumentDiagnosticReportResult::Report(
            DocumentDiagnosticReport::Full(RelatedFullDocumentDiagnosticReport {
                related_documents: None,
                full_document_diagnostic_report: FullDocumentDiagnosticReport {
                    result_id: Some(result_id),
                    items,
                },
            }),
        ))
    }

    async fn range_formatting(
        &self,
        params: DocumentRangeFormattingParams,
    ) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;
        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };
        let edits = crate::formatting::format_range(&doc, params.range);
        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits))
        }
    }

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;

        let doc = match self.state.snapshot(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let edits = crate::formatting::format_document(&doc);
        if edits.is_empty() {
            Ok(None)
        } else {
            Ok(Some(edits))
        }
    }
}
