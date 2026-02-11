use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::semantic_tokens;
use crate::state::ServerState;

pub struct LogicAffeineServer {
    client: Client,
    state: ServerState,
}

impl LogicAffeineServer {
    pub fn new(client: Client) -> Self {
        LogicAffeineServer {
            client,
            state: ServerState::new(),
        }
    }

    async fn publish_diagnostics(&self, uri: Url) {
        if let Some(doc) = self.state.documents.get(&uri) {
            self.client
                .publish_diagnostics(uri.clone(), doc.diagnostics.clone(), Some(doc.version))
                .await;
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for LogicAffeineServer {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: semantic_tokens::legend(),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: None,
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
                        "with".to_string(),
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
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        self.state.open_document(
            params.text_document.uri,
            params.text_document.text,
            params.text_document.version,
        );
        self.publish_diagnostics(uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        // We use FULL sync, so there's exactly one change with the full text
        if let Some(change) = params.content_changes.into_iter().next() {
            self.state.update_document(
                &uri,
                change.text,
                params.text_document.version,
            );
        }
        self.publish_diagnostics(uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.state.close_document(&params.text_document.uri);
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
        let doc = match self.state.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let tokens = semantic_tokens::encode_tokens(&doc.tokens, &doc.line_index);

        Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: tokens,
        })))
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = &params.text_document.uri;
        let doc = match self.state.documents.get(uri) {
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

        let doc = match self.state.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::definition::goto_definition(&doc, position, uri))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let doc = match self.state.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::hover::hover(&doc, position))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let doc = match self.state.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::completion::completions(&doc, position))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let doc = match self.state.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        let include_declaration = params.context.include_declaration;
        let locations =
            crate::references::find_references(&doc, position, uri, include_declaration);

        if locations.is_empty() {
            Ok(None)
        } else {
            Ok(Some(locations))
        }
    }

    async fn signature_help(&self, params: SignatureHelpParams) -> Result<Option<SignatureHelp>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let doc = match self.state.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::signature_help::signature_help(&doc, position))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        let range = params.range;

        let doc = match self.state.documents.get(uri) {
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

        let doc = match self.state.documents.get(uri) {
            Some(doc) => doc,
            None => return Ok(None),
        };

        Ok(crate::rename::rename(&doc, position, params.new_name, uri))
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> Result<Option<PrepareRenameResponse>> {
        let uri = &params.text_document.uri;
        let position = params.position;

        let doc = match self.state.documents.get(uri) {
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

        let doc = match self.state.documents.get(uri) {
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

        let doc = match self.state.documents.get(uri) {
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

        let doc = match self.state.documents.get(uri) {
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

    async fn formatting(&self, params: DocumentFormattingParams) -> Result<Option<Vec<TextEdit>>> {
        let uri = &params.text_document.uri;

        let doc = match self.state.documents.get(uri) {
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
