// Copyright 2026 the Message Format Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! CLI for running WG JSON tests and printing pass/fail scoreboard rows.

use std::path::PathBuf;

use message_format_conformance::harness::{default_wg_root, run_wg_suite_dir};

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_wg_root);
    let tests_dir = root.join("test/tests");

    let reports = match run_wg_suite_dir(&tests_dir) {
        Ok(reports) => reports,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    };

    let mut total = 0_usize;
    let mut passed = 0_usize;
    let mut failed = 0_usize;

    for item in &reports {
        println!(
            "{}\tpassed={}\tfailed={}\ttotal={}",
            item.path, item.report.passed, item.report.failed, item.report.total
        );
        total += item.report.total;
        passed += item.report.passed;
        failed += item.report.failed;
    }

    println!("SUMMARY\tpassed={passed}\tfailed={failed}\ttotal={total}");
}
