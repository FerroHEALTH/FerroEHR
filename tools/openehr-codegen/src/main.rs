// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `openehr-codegen` binary entry point over [`openehr_codegen::run`].

fn main() -> std::process::ExitCode {
    openehr_codegen::run()
}
