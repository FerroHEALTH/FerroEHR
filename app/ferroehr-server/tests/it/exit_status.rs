// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The exit status of the `ferroehr` run path (`ferroehr_server::process::main`): a
//! refused configuration exits 78, `EX_CONFIG` of `sysexits(3)`, and every
//! other failure exits 1.

use std::net::TcpListener;
use std::process::ExitCode;

use ferrohealth_server::run::{EXIT_CONFIG, EXIT_FAILURE};

#[test]
fn a_refused_configuration_exits_78() {
    let code =
        ferroehr_server::process::main(["ferroehr", "--set", "nonsense.key=1", "config", "check"]);
    assert_eq!(ExitCode::from(EXIT_CONFIG.get()), code);
}

#[test]
fn a_failed_probe_exits_1() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let address = listener.local_addr().expect("the bound address");
    drop(listener);
    let listen = format!("server.listen={address}");
    let code =
        ferroehr_server::process::main(["ferroehr", "--set", listen.as_str(), "healthcheck"]);
    assert_eq!(ExitCode::from(EXIT_FAILURE.get()), code);
}
