// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ferroehr definitions load`: its command line and the format each file's
//! extension names. The load itself is tested where it lives
//! (`app/ferroehr/tests/it/definition_load.rs`).

use std::path::Path;

use clap::Parser as _;
use ferroehr::service::definition::load::SourceFormat;
use ferroehr_server::Cli;
use ferroehr_server::definitions::format_of;

/// The subcommand takes its files in the order given, and needs one.
#[test]
fn load_takes_its_files_in_order() {
    let cli = Cli::try_parse_from(["ferroehr", "definitions", "load", "person.adls", "role.opt"])
        .expect("two files parse");
    let parsed = format!("{cli:?}");
    let person = parsed.find("person.adls").expect("the first file is kept");
    let role = parsed.find("role.opt").expect("the second file is kept");
    assert!(person < role, "{parsed}");
    assert!(Cli::try_parse_from(["ferroehr", "definitions", "load"]).is_err());
}

/// `.opt` is OPT 1.4 and `.adls` ADL 2, in any case; any other extension is
/// refused with the file named.
#[test]
fn the_extension_names_the_format() {
    assert_eq!(
        format_of(Path::new("a/role.opt")).expect("an OPT"),
        SourceFormat::Opt14
    );
    assert_eq!(
        format_of(Path::new("a/person.ADLS")).expect("an ADL 2 source"),
        SourceFormat::Adl2
    );
    for refused in ["a/person.adl", "a/person.json", "a/person"] {
        let error = format_of(Path::new(refused)).expect_err("refused");
        assert!(error.to_string().contains(refused), "{error}");
    }
}
