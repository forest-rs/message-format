// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Language Server Protocol implementation for `MessageFormat` 2.

use lsp_server::Connection;
use lsp_types::InitializeParams;

mod analysis;
mod capabilities;
mod diagnostics;
mod document;
mod manifest;
mod navigate;
mod scanner;
mod server;

fn main() {
    env_logger::init();

    let (connection, io_threads) = Connection::stdio();

    let (id, params) = connection
        .initialize_start()
        .expect("failed to start initialization");

    let params: InitializeParams =
        serde_json::from_value(params).expect("failed to parse InitializeParams");

    let capabilities = capabilities::server_capabilities();
    let init_result = serde_json::to_value(lsp_types::InitializeResult {
        capabilities,
        server_info: Some(lsp_types::ServerInfo {
            name: String::from("message-format-lsp"),
            version: Some(String::from(env!("CARGO_PKG_VERSION"))),
        }),
    })
    .expect("failed to serialize InitializeResult");

    connection
        .initialize_finish(id, init_result)
        .expect("failed to finish initialization");

    let mut srv = server::Server::new(connection, &params);
    srv.run();

    io_threads.join().expect("failed to join IO threads");
}
