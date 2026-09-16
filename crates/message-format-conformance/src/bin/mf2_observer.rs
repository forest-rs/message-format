// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! JSON-lines adapter for differential tests.

use std::io::{self, BufRead, Write};

use message_format_conformance::differential::{DifferentialCase, observe};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for (index, line) in stdin.lock().lines().enumerate() {
        let line = line.map_err(|error| format!("read case {}: {error}", index + 1))?;
        if line.trim().is_empty() {
            continue;
        }
        let case: DifferentialCase = serde_json::from_str(&line)
            .map_err(|error| format!("parse case {}: {error}", index + 1))?;
        serde_json::to_writer(&mut stdout, &observe(&case))
            .map_err(|error| format!("write observation {}: {error}", index + 1))?;
        stdout
            .write_all(b"\n")
            .map_err(|error| format!("write observation {}: {error}", index + 1))?;
    }
    stdout.flush().map_err(|error| format!("flush: {error}"))
}
