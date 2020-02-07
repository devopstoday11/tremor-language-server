// Copyright 2018-2020, Wayfair GmbH
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::{language, lsp_utils};
use futures::future;
use halfbrown::HashMap;
use jsonrpc_core::{BoxFuture, Result};
use serde_json::Value;
use std::fs;
use std::sync::Mutex;
use tower_lsp::lsp_types::*;
use tower_lsp::{LanguageServer, Printer};

// stores the latest state of the document as it changes (on edits)
// TODO can add more fields here based on ast parsing
#[derive(Debug, Default)]
struct DocumentState {
    text: String,
}

// mapping of file uri to its server document state
type State = HashMap<Url, DocumentState>;

pub struct Backend {
    language: Box<dyn language::Language>,
    state: Mutex<State>,
}

impl Backend {
    pub fn new(language: Box<dyn language::Language>) -> Self {
        Self {
            language,
            state: Mutex::new(State::new()),
        }
    }

    fn update(&self, uri: Url, text: &str) {
        // TODO implement update as well. also remove unwrap
        self.state.lock().unwrap().insert(
            uri,
            DocumentState {
                text: text.to_string(),
            },
        );
    }

    // LSP helper functions

    fn get_diagnostics(&self, text: &str) -> Vec<Diagnostic> {
        file_dbg("get_diagnostics", text);

        let mut diagnostics = Vec::new();

        if let Some(errors) = self.language.parse_errors(text) {
            for e in &errors {
                let range = Range {
                    start: lsp_utils::to_lsp_position(&e.start),
                    end: lsp_utils::to_lsp_position(&e.end),
                };

                let mut message = e.callout.to_string();
                if let Some(hint) = &e.hint {
                    // comma here splits the message into multiple lines
                    message = format!("{}, Note: {}", message, hint);
                }

                diagnostics.push(Diagnostic {
                    range,
                    message,
                    severity: Some(lsp_utils::to_lsp_severity(&e.level)),
                    source: Some("tremor-language-server".to_string()),
                    code: None,
                    related_information: None,
                });
            }
        }

        diagnostics
    }

    fn get_completions(&self, text: &str, position: Position) -> Vec<CompletionItem> {
        if let Some(token) = lsp_utils::get_token(text, position) {
            file_dbg("get_completions_token", &token);
            let module_parts: Vec<&str> = token.rsplitn(2, "::").collect();

            if let Some(module_name) = module_parts.get(1) {
                file_dbg("get_completions_module_name", module_name);
                return self
                    .language
                    .functions(module_name)
                    .iter()
                    .map(|function_name| {
                        let mut detail = None;
                        let mut documentation = None;
                        let mut insert_text = None;
                        if let Some(function_doc) = self
                            .language
                            .function_doc(&format!("{}::{}", module_name, function_name))
                        {
                            file_dbg("get_completions_function_doc", &function_doc.description);
                            detail = Some(function_doc.signature.to_string());
                            documentation = Some(Documentation::MarkupContent(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: function_doc.description.clone(),
                            }));
                            let args_snippet = function_doc
                                .signature
                                .args
                                .iter()
                                .enumerate()
                                // produces snippet text like ${1:arg} (where arg is the placeholder text)
                                // https://microsoft.github.io/language-server-protocol/specifications/specification-3-14/#snippet-syntax
                                .map(|(i, arg)| format!("${{{}:{}}}", i + 1, arg))
                                .collect::<Vec<String>>()
                                .join(", ");
                            insert_text = Some(format!("{}({})", function_name, args_snippet));
                        };
                        CompletionItem {
                            label: function_name.to_string(),
                            kind: Some(CompletionItemKind::Function),
                            detail,
                            documentation,
                            insert_text,
                            insert_text_format: Some(InsertTextFormat::Snippet),
                            ..CompletionItem::default()
                        }
                    })
                    .collect();
            }
        }

        vec![]
    }

    fn get_hover_content(&self, text: &str, position: Position) -> Option<MarkupContent> {
        if let Some(token) = lsp_utils::get_token(text, position) {
            file_dbg("get_hover_content_token", &token);
            if token.contains("::") {
                if let Some(function_doc) = self.language.function_doc(&token) {
                    file_dbg("get_hover_content_function_doc", &function_doc.description);
                    return Some(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: function_doc.to_string(),
                    });
                }
            }
        }
        None
    }
}

impl LanguageServer for Backend {
    type ShutdownFuture = BoxFuture<()>;
    type SymbolFuture = BoxFuture<Option<Vec<SymbolInformation>>>;
    type ExecuteFuture = BoxFuture<Option<Value>>;
    type CompletionFuture = BoxFuture<Option<CompletionResponse>>;
    type HoverFuture = BoxFuture<Option<Hover>>;
    type HighlightFuture = BoxFuture<Option<Vec<DocumentHighlight>>>;

    fn initialize(&self, _: &Printer, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                code_action_provider: None,
                code_lens_provider: None, /*Some(CodeLensOptions {
                                              resolve_provider: None,
                                          }),*/
                color_provider: None,
                completion_provider: Some(CompletionOptions {
                    resolve_provider: None,
                    trigger_characters: Some(vec![":".to_string()]),
                }),
                definition_provider: None,
                document_formatting_provider: None,
                document_highlight_provider: None,
                document_link_provider: None,
                document_on_type_formatting_provider: None,
                document_range_formatting_provider: None,
                document_symbol_provider: None,
                execute_command_provider: None,
                folding_range_provider: None,
                hover_provider: Some(true),
                implementation_provider: None,
                references_provider: None,
                rename_provider: None,
                signature_help_provider: None,
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::Full,
                )),
                type_definition_provider: None,
                workspace_symbol_provider: None,
                workspace: Some(WorkspaceCapability {
                    workspace_folders: Some(WorkspaceFolderCapability {
                        supported: Some(true),
                        change_notifications: Some(
                            WorkspaceFolderCapabilityChangeNotifications::Bool(true),
                        ),
                    }),
                }),
            },
        })
    }

    fn initialized(&self, printer: &Printer, _: InitializedParams) {
        file_dbg("initialized", "initialized");
        // TODO check this from clients
        //printer.show_message(MessageType::Info, "Initialized Trill!");
        printer.log_message(MessageType::Info, "Initialized Trill!");
    }

    // TODO do more here (as appropriate). manadatory implementations for the trait

    fn shutdown(&self) -> Self::ShutdownFuture {
        file_dbg("shutdown", "shutdown");
        Box::new(future::ok(()))
    }

    fn symbol(&self, _: WorkspaceSymbolParams) -> Self::SymbolFuture {
        file_dbg("symbol", "symbol");
        Box::new(future::ok(None))
    }

    fn document_highlight(&self, _: TextDocumentPositionParams) -> Self::HighlightFuture {
        file_dbg("document_highlight", "document_highlight");
        Box::new(future::ok(None))
    }

    fn execute_command(&self, printer: &Printer, _: ExecuteCommandParams) -> Self::ExecuteFuture {
        file_dbg("execute", "execute");
        printer.log_message(MessageType::Info, "executing command!");
        Box::new(future::ok(None))
    }

    // backend state updates on text edits and reporting of diagnostics

    fn did_open(&self, printer: &Printer, params: DidOpenTextDocumentParams) {
        file_dbg("didOpen", "didOpen");
        file_dbg("didOpen_language", &params.text_document.language_id);

        let uri = params.text_document.uri;
        if let Ok(path) = uri.to_file_path() {
            // TODO pull this from params.text_document.text
            // TODO cleanup
            if let Ok(text) = fs::read_to_string(path) {
                self.update(uri.clone(), &text);
                printer.publish_diagnostics(uri, self.get_diagnostics(&text));
            }
        }
    }

    fn did_change(&self, printer: &Printer, params: DidChangeTextDocumentParams) {
        file_dbg("didChange", "didChange");
        // TODO cleanup
        let uri = params.text_document.uri;
        let text = &params.content_changes[0].text;
        self.update(uri.clone(), text);
        printer.publish_diagnostics(uri, self.get_diagnostics(text));
    }

    fn did_close(&self, printer: &Printer, params: DidCloseTextDocumentParams) {
        file_dbg("didClose", "didClose");
        // TODO can cleanup backend state here
        printer.publish_diagnostics(params.text_document.uri, vec![]);
    }

    // other lsp features

    fn completion(&self, params: CompletionParams) -> Self::CompletionFuture {
        file_dbg("completion", "completion");

        // TODO remove unwraps
        let state = self.state.lock().unwrap();
        let doc = state
            .get(&params.text_document_position.text_document.uri)
            .unwrap();

        Box::new(future::ok(Some(CompletionResponse::Array(
            self.get_completions(&doc.text, params.text_document_position.position),
        ))))
    }

    fn hover(&self, params: TextDocumentPositionParams) -> Self::HoverFuture {
        file_dbg("hover", "hover");
        // TODO remove unwraps
        // TODO bake state lookup in self
        let state = self.state.lock().unwrap();
        let doc = state.get(&params.text_document.uri).unwrap();

        let result = self
            .get_hover_content(&doc.text, params.position)
            .map(|hover_content| Hover {
                contents: HoverContents::Markup(hover_content),
                range: None,
            });

        Box::new(future::ok(result))
    }
}

// TODO remove. just for testing right now
pub fn file_dbg(name: &str, content: &str) {
    use std::fs::File;
    use std::io::Write;

    let path = format!("/tmp/tremor_{}", name);

    let mut output = File::create(path).unwrap();
    write!(output, "{}", content);
}