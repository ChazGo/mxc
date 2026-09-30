// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! `policy-catalog` command-line harness (PROTOTYPE). See `mxc_policy_catalog::cli`.

use std::io::Write;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let output = mxc_policy_catalog::cli::run(&argv);
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(output.stdout.as_bytes());
    let _ = stdout.flush();
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(output.stderr.as_bytes());
    let _ = stderr.flush();
    ExitCode::from(output.exit_code as u8)
}
