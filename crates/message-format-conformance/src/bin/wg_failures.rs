// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! CLI for listing failing WG cases in a JSON suite file.

use std::path::PathBuf;

use message_format_conformance::harness::run_wg_json_file_cases;

fn main() {
    let Some(path) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: wg_failures <path-to-wg-json-file>");
        std::process::exit(2);
    };

    let cases = match run_wg_json_file_cases(&path) {
        Ok(cases) => cases,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    };

    let mut failed = 0_usize;
    for case in cases {
        if case.passed {
            continue;
        }
        failed += 1;
        println!("{}:\t{}\t({})", case.index, case.src, case.detail);
    }

    if failed == 0 {
        println!("no failing cases");
    }
}
