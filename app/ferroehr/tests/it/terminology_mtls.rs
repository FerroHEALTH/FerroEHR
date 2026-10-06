// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Mutual TLS from the CDR to a terminology server, proven against a real TLS
//! listener that demands (and inspects) the client certificate.
//!
//! `wiremock` cannot terminate TLS, let alone require a client certificate, so
//! these tests stand up a minimal `rustls` acceptor in-process and answer one
//! canned FHIR `CodeSystem/$lookup` over it. What is asserted is the handshake
//! outcome and the certificate the server actually saw — the only evidence
//! that the client identity was presented rather than merely configured.
//!
//! No openEHR spec governs terminology-server transport security — our own
//! design/extension (`BASE/docs/architecture_overview/master12-terminology.adoc`
//! models the backend only as an external "terminology query server").
//!
//! # The test PKI
//!
//! Generated once, offline, with `openssl` — the same approach as the ATNA TLS
//! round-trip test, and for the same reason: no certificate-minting crate is in
//! the workspace dependency set, and the repository's `.gitignore` deliberately
//! refuses committed `*.pem` / `*.key` files. The material is therefore inlined
//! here and written to a temporary directory per test, because the
//! configuration keys under test are *paths*.
//!
//! - CA `CN=ferroehr-terminology-test-ca` (`basicConstraints=CA:TRUE`);
//! - server leaf `CN=localhost`, SAN `DNS:localhost` + `IP:127.0.0.1`,
//!   `extendedKeyUsage=serverAuth`;
//! - client leaf `CN=ferroehr-cdr`, `extendedKeyUsage=clientAuth`.
//!
//! Both leaves are signed by the CA and expire in 2126, so the suite does not
//! rot. Every key here is a throwaway that never left this repository's tests.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ferroehr::service::status::{CallStatusType, SmError};
use ferroehr::service::terminology::config::{
    ExternalTerminologyConfig, FhirOperation, FhirProviderConfig, ProviderKind,
};
use ferroehr::service::terminology::router::TerminologyRouter;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

const CA_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIDPzCCAiegAwIBAgIUQdzLptxYSE4Z2KUtCRX4orOs5x4wDQYJKoZIhvcNAQEL
BQAwJjEkMCIGA1UEAwwbZWhyYmFzZS10ZXJtaW5vbG9neS10ZXN0LWNhMCAXDTI2
MDcyOTEzNDQ0MFoYDzIxMjYwNzA1MTM0NDQwWjAmMSQwIgYDVQQDDBtlaHJiYXNl
LXRlcm1pbm9sb2d5LXRlc3QtY2EwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEK
AoIBAQCr5GwUx1SYbKWyobe/4gNljimQ2fMkF6NJ2UbqQo1YmchqjbDU3yqPhO+u
2uA3tjPy7LJiyFai7JexYbrgBcQEcphDvfWfTUTkpb5XxSrWYBxQEW1auxWpq33m
5oQKch+SkblKpdhGXnRwBwtmiQs/oECPexEq/AOWCYwjn0Y0ZwZryjH0L0SAlHXa
0SWIihVz2pBGk1xKiQkV7OPM+sbbR1e9D+clQOzTHWz1cGtfep7b2vWn14nBc36C
cbF336umG/2XiF9B+dFM6KQxNRA3RizCuSPIIH0fdQBaAVuCzjGc3/sLqvulsbxV
cEFmZchu7kEcVGVdcrS1tCple8AFAgMBAAGjYzBhMB0GA1UdDgQWBBT4JtIe+Kky
uKi1rzSo2Sc1sg+zhjAfBgNVHSMEGDAWgBT4JtIe+KkyuKi1rzSo2Sc1sg+zhjAP
BgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBBjANBgkqhkiG9w0BAQsFAAOC
AQEAhD4fgejqDAO0ypzfEgJLzaFqqvJpbt8pyiNZ+THR6KrjylW3pUNk1WZRxL9p
4b9A1YKtQX1ABt2AEJ0pFgm1PiXy4EDszyRK8A33hls91c6761pHds0+SFYsq6jr
zSJEanbJ5sQccTbsZ6nkzEjq3y/QBmw49TAqzbzMrYJjvHFPXpUr3Bpt9qTR/UmQ
NQ/kWfCpLqmpWD8O71enKN1F7UId3nnpEus+/HMtnPtErMHBfGBOWsr95d37NB52
CJTogkq19bUHC9nI4XBnNhR4F1BTZL9lQYghjettRzu0fvHj0JCR437GbkFmOZyA
6O1x7xRNYxQG+wohHHPFnyDENg==
-----END CERTIFICATE-----
";

const SERVER_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIDWjCCAkKgAwIBAgIUN6RUnq7anOZdiSdi9b6onYdW7PAwDQYJKoZIhvcNAQEL
BQAwJjEkMCIGA1UEAwwbZWhyYmFzZS10ZXJtaW5vbG9neS10ZXN0LWNhMCAXDTI2
MDcyOTEzNDQ0MFoYDzIxMjYwNzA1MTM0NDQwWjAUMRIwEAYDVQQDDAlsb2NhbGhv
c3QwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCbjXXyGI544/qF09KL
GXWCNXr4mN4DXDfuSGNGpMXV+maRhkK3rA5c+bphPmwaR6RQ4Bc7y0bPITqVU8AF
m4VA0MZ2qe2TTKr8eaBCBZXETXLH3NjQuARVgO05hgrNSIgTFYhRrUW0nJo9SYZ9
ryAYhpokhilrvquziyBMlhZzk+QoqUclnCDYdjJsTR8dSVxrKX0qmyN4kTjBCpKt
y5Me7SbX0xy/AT2lPlUIkAkAxkwcrbZ0cB7BCvR7xdFE49GSc/3s5G8dFujsywNk
hMCE0TbUMvgodTURJN9VD3Kw9bypJq7At6IFGrMX1oDepGGno48Ke7XlgRBTjyDA
4xWdAgMBAAGjgY8wgYwwGgYDVR0RBBMwEYIJbG9jYWxob3N0hwR/AAABMA4GA1Ud
DwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAJBgNVHRMEAjAAMB0GA1Ud
DgQWBBTdRR/Wxur642w7IpXvYTI0OhMuxzAfBgNVHSMEGDAWgBT4JtIe+KkyuKi1
rzSo2Sc1sg+zhjANBgkqhkiG9w0BAQsFAAOCAQEAFjeZyyNnvB3Hof0KFw/DMkV3
xdk7ls7DPyfT0GUDiKtYP8wuMNpEjavvz4KmsE9Dri7xTgy6SiROLF5bQav7t4LN
eeLZu47XX+8+BZNLmWv6OUU0Z9uzDZ4XR62jjac4Zoiu+isJGkZl9QMoeJJxln+d
yBRARFGNmsDK/Db3Oi/PJE7FtiSda1mnxua8sRTBjujizcsF0jRoUCtc9hPFSEdY
65qmTdb7rJUxTA6yFj1Hp1M6WzeuNGLZnoMV9c5QF50Cvkg29PVd+C57gRHYqAui
T4eTPOd62IGoTZeAe+jQrFm5xdV6Q7wic93uLRJdYnJ7ZZKGuEzIDqWF2yDyrQ==
-----END CERTIFICATE-----
";

const SERVER_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCbjXXyGI544/qF
09KLGXWCNXr4mN4DXDfuSGNGpMXV+maRhkK3rA5c+bphPmwaR6RQ4Bc7y0bPITqV
U8AFm4VA0MZ2qe2TTKr8eaBCBZXETXLH3NjQuARVgO05hgrNSIgTFYhRrUW0nJo9
SYZ9ryAYhpokhilrvquziyBMlhZzk+QoqUclnCDYdjJsTR8dSVxrKX0qmyN4kTjB
CpKty5Me7SbX0xy/AT2lPlUIkAkAxkwcrbZ0cB7BCvR7xdFE49GSc/3s5G8dFujs
ywNkhMCE0TbUMvgodTURJN9VD3Kw9bypJq7At6IFGrMX1oDepGGno48Ke7XlgRBT
jyDA4xWdAgMBAAECggEAGN7s2UkI2pZk82nTU48+BRQk9cOHV9UyUiR7zwtAYH6Z
ULI9T52wbDg3jx3Kbgc/Y/j4bgSJ7Us8USzjWmIr05mt6cIwrKkI+7Y8o+G9uPXD
IOaUOgb6Fmu3QkfGyVzL+PUr5xdDumWBmcP8P3M1OAapdaaHz+TcEihwAR7MNy1R
47BG24EtO7RWLOOdNsm3wYqMdCvoXJ8U9wPudigSKi7m+oO4+uTdmFslf1uDc2ie
QUXRehjvof/3w/ltWJI/F+4DO1ze6deSIUz3G0c6dbENU9wmHGRgmMjBH1Ksiqqy
xczsq4wt6/F/HcYBCeq/av/P2u7bGfeT9316E+kJxwKBgQDPx4kfm51k5HQi2nRe
0c4ODVDu6P5R8P1h+q2VmPHyWoZGO9s+8b11FVSi1EyNPS5gzpltmeetQkSMiAs2
unSlkpe4UG8cr2X2T2DIpEOgQL3WNWuKO61+W04AKXlnTacF2TJh+ecwp8xpQvVs
Qhdxo1pP/5tCHwSbozXgF58n0wKBgQC/pw4irsApIDn2OrG+nIuZutXCKq/++HMy
cyEiHd1Fx6+qX+AVPrz8XNWB6o4ZsBJHGEfKjCw5Fe1RcLmuAASveG/bAnei3L4a
vvTUJaviM3M7XXnAdsdfOBKCkzDOSjkkngyY5Frel6YkB73X6iknPKes8HMS7E0J
coomR0tWzwKBgQCh1xMIqqZLGuMW7r9rx9HPAjJDFPpbCvHiKmagunPiSP6DoEXi
3lqq4wV8mw5RiREh2GqLgzCAtLg+Gg1aAJuxB+DjcMtLNZee5i9FuSTvot37BrsP
/fHiFO5JlAR7IXHyTT5AMG4SaPEAIGaXf1dRbWKAI2FkfFKTg+oH9X5DfwKBgEoy
Roqu1L4XN9lXx9BfkrwlVPQiypgPX6m8YKtwnGWTdTKkg4A2FbwtxIrTX8gaHjlf
8Qs9UTGYh5Pr7Das0yOLoOJNBjwK8Z4xJ1+qZezgtk/ZVHVqhq0abDAZA+AZZB4F
AiN+5J8gXrW8OYcJpH0IQnH1dNdynDB4I3vGRiiJAoGAJALnmQ2t263WmSt+pNRM
jIrhr8CJwPiv648qLu+xkWNVjHqX5JqOT7LPYinSoNjEWfo5o6Bl52lgcwE/B2j4
+flFd9WfI18fXESLmKfzisARe+iftNsO75rwpFu/AZOaizFg/wqZlnaNCl3+No5n
VZeOC1RrGYXZFXe6/XlN/Y4=
-----END PRIVATE KEY-----
";

const CLIENT_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIDPjCCAiagAwIBAgIUN6RUnq7anOZdiSdi9b6onYdW7PEwDQYJKoZIhvcNAQEL
BQAwJjEkMCIGA1UEAwwbZWhyYmFzZS10ZXJtaW5vbG9neS10ZXN0LWNhMCAXDTI2
MDcyOTEzNDQ0MVoYDzIxMjYwNzA1MTM0NDQxWjAWMRQwEgYDVQQDDAtlaHJiYXNl
LWNkcjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAKYtID4ud0jEkUFr
Dync/gEVvAeklhwA23JEdsLobmJkFJtbn68qJmmEMRyJx/0OYNwpoJ8MeY7NMNyx
63J6lDfmhQFpxJVxHqLspBp9DmEu0ifO7Q5VmEeQrnqkd4cF/hp61M7X83TybG79
O1605+X30/amgxHj6Y3NAFuO3+C9ayJl8oFjrX86if9m4ywPO2pps8fKRmy6V0P1
O2PBIaf8lEvPYbzEj8pMiBiWj45ZuNopOq5OAUIndFtR55G7cL51KrocMuVo48i4
AXRQQR14BDyhiyuCd8zhDtMoGuWsa0E+K9yove6Uhv7aKyaVlkVPSQ9vIQjXx9LJ
b2n0QRkCAwEAAaNyMHAwDgYDVR0PAQH/BAQDAgeAMBMGA1UdJQQMMAoGCCsGAQUF
BwMCMAkGA1UdEwQCMAAwHQYDVR0OBBYEFL65rmBZ4vWcx18igZhEiWnsGDp/MB8G
A1UdIwQYMBaAFPgm0h74qTK4qLWvNKjZJzWyD7OGMA0GCSqGSIb3DQEBCwUAA4IB
AQAHzzukoZnHp6zkcSwulSMUsECfdXQFjmPszTsgdoMRtozrIJBeTTn8TU4udt83
lcqJO2aHxaEsXQAC6HiO68LLqudio4BymkpYY/jnBLZNOCiChY2NkMR+PNnWrrKh
eXBNKgzE1OFcPs2eDzXDGi4u0C+5TAzTyk1iFJSjVPrm+WYBlSQ40+KkZjc3QPqM
9mmbv/ywOeaLihcgIeIkTHQLb7dlhTazDyX5KpqegPqh/NMXVdVchN0pnRZ7l4V9
xYMKapLu7dF65h/aXFYQaKSn7ztjDS5/sEKcwBjjSRcoXOqISuXyfONEjuInj/Lh
WKGF8hUt9DgUqWPuDe33JrC8
-----END CERTIFICATE-----
";

const CLIENT_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCmLSA+LndIxJFB
aw8p3P4BFbwHpJYcANtyRHbC6G5iZBSbW5+vKiZphDEcicf9DmDcKaCfDHmOzTDc
setyepQ35oUBacSVcR6i7KQafQ5hLtInzu0OVZhHkK56pHeHBf4aetTO1/N08mxu
/TtetOfl99P2poMR4+mNzQBbjt/gvWsiZfKBY61/Oon/ZuMsDztqabPHykZsuldD
9TtjwSGn/JRLz2G8xI/KTIgYlo+OWbjaKTquTgFCJ3RbUeeRu3C+dSq6HDLlaOPI
uAF0UEEdeAQ8oYsrgnfM4Q7TKBrlrGtBPivcqL3ulIb+2ismlZZFT0kPbyEI18fS
yW9p9EEZAgMBAAECggEACfes9m3dE81OlSjxyOYLik8ebyrtIhLfFtSKdxhv/pDY
N5VgV6ZklXGrbHXLPB+PqcUJcGDULb+bDbHSWJSHrW6zTalldD1LxCQDl98mbKfd
TSv5RiHWN3yzKoIQ9VVjr3zspNeJL9uWq3WfCQg63K1n1mSYegs8qBfCzLseLKG3
LR2yhZSbMdH7NNWwoG1iwDXyJGx4SM4r7z2dPRobMxT5ErIKtCzDRLcUWMeW3FiU
qEB5SXscrF5TitQaFtwRFPO0IpbPWoXbW99OZTPoi+ZHUxVRShWE3gtO3D6cvgHf
iwLxdpy3tUHIAnm5SbDjtp30AAF1S/CUU3ZpcoiafQKBgQDc7/LI4FcG0xmKzGHe
vzfDZDUFxjVkCY0zQ919l9TufJZbZXOp6iVnAEts46ykZ2oPZXAt9csLT94EF4iy
1/VxQ67NQxAyfMMKCCTMMgAuL0VCWhzKdx0udcOvoqdnoRAt48jOW/9fKlhSitMW
Ef6pg+n1T3ZbXUPE+zi4ezC+vwKBgQDAjGS6k11728mqKP9SThpGGdPiJVwM9c3y
etCHwz0g7Dfo0dZBFrXbyDksaI+M/OchbJLq3Ar+UOlB0l08m60f6OmXVzSDFvgs
m1ilCvAp3cxvFuasji5CqTlQ5DZlNbjbsADUHwel4geiCgYvn655IONuUqo2stCf
qArTp9dOJwKBgQCo/ML3kFggOTDtL/yf0iRFyAyiOQO3W3LrxjnQiWRtcU/T4lpA
mX44NUp7o/z11r+RvSW7kafXJCSNfq6pFHOASaOXDneCFllb//SdVpU6vh88bA5f
chIY6ixd14wxwEjOwM5jwIwobwwVPmfMFsFxSRuW7Ut7AHAIZ5rvyBH1owKBgQCB
yWBQPurhhPm+/9lyEgE1xU0D/2i3t6v1SQFssZZvranV/jMsNnGozqJzI5u3TfVB
m1zAgEfMup8v5etA4jJk8usZPwe/YOkxsBilTuUpYz7clpQwNbpK5qQiuWFNAVQ0
iMNWOABAuUWp3JXk3f6N2TRT9daT/h4PsAZ0OosvOwKBgQCt7E+/RlRNlcC+4o2S
kT6qEL4sTV0Qc+RMtlakqDaLElcQx9ZIc4SPAeiIRq0Lm/w5l1GV1JvxawWR63P0
16krB8b1EGloCd5p8XBWMQRdn7Pyo+kY+Xrpo+3SJroCVbKHojJKxnPgX2GE0XVx
SxWyjGrM2xGr1RcYYQM5nopluQ==
-----END PRIVATE KEY-----
";

/// The one canned FHIR `CodeSystem/$lookup` answer the test server gives to
/// every request that reaches it — the payload is irrelevant here; that a
/// request reached it at all is the assertion.
const LOOKUP_BODY: &str = r#"{"resourceType":"Parameters","parameter":[{"name":"display","valueString":"Hypertension"}]}"#;

/// The temporary directory holding this test's PEM files, plus their paths —
/// the configuration keys under test are paths, so the material has to live on
/// disk. Dropping this removes the directory.
struct Pki {
    _dir: assert_fs::TempDir,
    ca: PathBuf,
    client_cert: PathBuf,
    client_key: PathBuf,
}

impl Pki {
    fn write() -> Self {
        let dir = assert_fs::TempDir::new().expect("temp dir");
        let write = |name: &str, bytes: &[u8]| {
            let path = dir.path().join(name);
            std::fs::write(&path, bytes).expect("write pem");
            path
        };
        let ca = write("ca.crt.pem", CA_CERT_PEM);
        let client_cert = write("client.crt.pem", CLIENT_CERT_PEM);
        let client_key = write("client.key.pem", CLIENT_KEY_PEM);
        Self {
            _dir: dir,
            ca,
            client_cert,
            client_key,
        }
    }
}

/// A running TLS terminology-server stub: its port, and the peer certificates
/// the first accepted connection presented (`None` when the handshake failed
/// or the client sent none).
struct TestTs {
    port: u16,
    peer: tokio::sync::mpsc::UnboundedReceiver<Option<usize>>,
}

impl TestTs {
    /// Start a TLS listener serving the `localhost` leaf. With `require_client`
    /// it also demands a certificate signed by the test CA, so a client
    /// without an identity is rejected at the handshake.
    async fn start(require_client: bool) -> Self {
        use rustls::pki_types::pem::PemObject;

        let mut roots = rustls::RootCertStore::empty();
        for cert in certs(CA_CERT_PEM) {
            roots.add(cert).expect("ca root");
        }
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let builder = rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()
            .expect("protocol versions");
        let builder = if require_client {
            let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
                Arc::new(roots),
                provider,
            )
            .build()
            .expect("client verifier");
            builder.with_client_cert_verifier(verifier)
        } else {
            builder.with_no_client_auth()
        };
        let mut config = builder
            .with_single_cert(
                certs(SERVER_CERT_PEM),
                rustls::pki_types::PrivateKeyDer::from_pem_slice(SERVER_KEY_PEM).expect("key"),
            )
            .expect("server cert");
        // The client is built with `default-features = false` (no `http2`), so
        // it offers `http/1.1`; declare it rather than leaving ALPN empty.
        config.alpn_protocols = vec![b"http/1.1".to_vec()];

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let (tx, peer) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let acceptor = acceptor.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        // A rejected handshake is itself the observation.
                        let _unused = tx.send(None);
                        return;
                    };
                    let presented = tls
                        .get_ref()
                        .1
                        .peer_certificates()
                        .map(<[rustls::pki_types::CertificateDer<'_>]>::len);
                    let _unused = tx.send(presented);
                    // Read the request head, then answer it.
                    let mut buf = [0_u8; 2048];
                    let _read = tls.read(&mut buf).await;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/fhir+json\r\n\
                         content-length: {}\r\nconnection: close\r\n\r\n{LOOKUP_BODY}",
                        LOOKUP_BODY.len()
                    );
                    let _written = tls.write_all(response.as_bytes()).await;
                    let _flushed = tls.shutdown().await;
                });
            }
        });

        Self { port, peer }
    }

    fn url(&self) -> String {
        format!("https://localhost:{}/fhir", self.port)
    }
}

fn certs(pem: &[u8]) -> Vec<rustls::pki_types::CertificateDer<'static>> {
    use rustls::pki_types::pem::PemObject;
    rustls::pki_types::CertificateDer::pem_slice_iter(pem)
        .collect::<Result<_, _>>()
        .expect("certificates")
}

/// A provider config pointing at `url`, cache off so every call is a real
/// connection.
fn provider_cfg(url: &str) -> FhirProviderConfig {
    FhirProviderConfig {
        kind: ProviderKind::Fhir,
        url: ferroehr::config::secret::SecretUrl::new(url),
        operation: FhirOperation::ValidateCode,
        connect_timeout_ms: 2_000,
        request_timeout_ms: 4_000,
        oauth2_client: None,
        client_cert_path: None,
        client_key_path: None,
        ca_bundle_path: None,
        cache_ttl_secs: 0,
        cache_capacity: 0,
    }
}

fn external(cfg: FhirProviderConfig) -> ExternalTerminologyConfig {
    ExternalTerminologyConfig {
        enabled: true,
        providers: std::iter::once(("default".to_owned(), cfg)).collect(),
        ..ExternalTerminologyConfig::default()
    }
}

/// Build the router, take its sole provider, and run one `$lookup`.
async fn lookup(cfg: FhirProviderConfig) -> Result<bool, SmError> {
    let router = TerminologyRouter::build(&external(cfg))?.expect("router");
    let provider = router.default_provider().expect("default provider");
    provider
        .has_term("http://snomed.info/sct", "38341003", None)
        .await
}

/// (a) A provider carrying a client identity completes the handshake against a
/// server that demands one, and its request is answered. The server-side
/// assertion — a peer certificate was actually presented — is what
/// distinguishes "mutual TLS works" from "the connection happened to succeed".
#[tokio::test]
async fn a_client_identity_completes_the_mutual_tls_handshake() {
    let pki = Pki::write();
    let mut ts = TestTs::start(true).await;

    let mut cfg = provider_cfg(&ts.url());
    cfg.client_cert_path = Some(pki.client_cert.clone());
    cfg.client_key_path = Some(pki.client_key.clone());
    cfg.ca_bundle_path = Some(pki.ca.clone());

    assert!(
        lookup(cfg).await.expect("the mTLS lookup must succeed"),
        "the terminology server answered the lookup"
    );
    assert_eq!(
        ts.peer.recv().await.expect("the server saw a connection"),
        Some(1),
        "the server must have received exactly the configured client certificate"
    );
}

/// (b) The same server refuses a provider that configures no client identity:
/// the handshake fails and the SM call surfaces a transport exception rather
/// than a silent success.
#[tokio::test]
async fn the_same_server_refuses_a_provider_without_the_identity() {
    let pki = Pki::write();
    let mut ts = TestTs::start(true).await;

    let mut cfg = provider_cfg(&ts.url());
    // Trust anchors only — the server is verified, but nothing is presented.
    cfg.ca_bundle_path = Some(pki.ca.clone());

    let err = lookup(cfg)
        .await
        .expect_err("a client-authenticating server must refuse an anonymous client");
    // The failure is typed and loud (never a silent success), while the
    // OPERATOR detail — which configured provider, and the TLS diagnostic —
    // stays on the trace record and off the wire body.
    assert_eq!(err.status, CallStatusType::Exception, "got {err:?}");
    assert!(
        !err.message.contains("terminology provider 'default'"),
        "the configured provider must not be named on the wire, got {}",
        err.message
    );
    assert_eq!(
        ts.peer.recv().await.expect("the server saw a connection"),
        None,
        "the server must have rejected the handshake, seeing no client certificate"
    );
}

/// (c) A custom CA bundle is what makes a privately-issued terminology server
/// trusted: with `ca_bundle_path` the call succeeds, and with the platform's
/// default trust the very same server is refused. Verification is never
/// disabled — only the anchors change.
#[tokio::test]
async fn a_custom_ca_bundle_trusts_a_private_server_that_default_trust_refuses() {
    let pki = Pki::write();
    let ts = TestTs::start(false).await;

    let mut trusted = provider_cfg(&ts.url());
    trusted.ca_bundle_path = Some(pki.ca.clone());
    assert!(
        lookup(trusted).await.expect("the pinned-CA lookup"),
        "the private CA bundle makes the server trusted"
    );

    // Same server, same URL, no configured anchors: the default trust store
    // does not know this CA, so the connection is refused.
    //
    // NOTE: this arm is the slow one on macOS — default trust goes through the
    // OS certificate evaluation, which hunts for the unknown issuer
    // synchronously inside the handshake, past the provider's own timeouts.
    let untrusted = provider_cfg(&ts.url());
    let err = lookup(untrusted)
        .await
        .expect_err("default trust must refuse a privately-issued certificate");
    assert_eq!(err.status, CallStatusType::Exception, "got {err:?}");
    assert!(
        !err.message.contains("terminology provider 'default'"),
        "the configured provider must not be named on the wire, got {}",
        err.message
    );
}

/// (d) Broken TLS material is a BOOT failure, not a first-request surprise: the
/// router refuses to build. Both shapes are covered — a path that does not
/// exist, and a file that exists but holds the wrong PEM object.
///
/// The diagnosis is read off the CAUSE CHAIN
/// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)), not
/// off the message: the message is the one thing that could reach a client on
/// a `500`, so it names the provider and the failure class only, while the
/// `TlsMaterialError` naming the offending configuration key rides
/// [`std::error::Error::source`] for the operator's boot log.
#[tokio::test]
async fn broken_tls_material_fails_at_boot() {
    let pki = Pki::write();

    // A certificate path that does not exist.
    let mut missing = provider_cfg("https://ts.example.org/fhir");
    missing.client_cert_path = Some(PathBuf::from("/nonexistent/ferroehr-ts-client.crt.pem"));
    missing.client_key_path = Some(pki.client_key.clone());
    let err = TerminologyRouter::build(&external(missing))
        .expect_err("a missing certificate file must fail the build");
    assert_cause_names(&err, "client_cert_path");

    // A key path pointing at a certificate — readable, but not a private key.
    let mut wrong_key = provider_cfg("https://ts.example.org/fhir");
    wrong_key.client_cert_path = Some(pki.client_cert.clone());
    wrong_key.client_key_path = Some(pki.ca.clone());
    let err = TerminologyRouter::build(&external(wrong_key))
        .expect_err("a key file holding no private key must fail the build");
    assert_cause_names(&err, "no PEM private key");

    // A CA bundle path pointing at a private key — no trust anchor in it.
    let mut wrong_ca = provider_cfg("https://ts.example.org/fhir");
    wrong_ca.ca_bundle_path = Some(pki.client_key.clone());
    let err = TerminologyRouter::build(&external(wrong_ca))
        .expect_err("a CA bundle holding no certificate must fail the build");
    assert_cause_names(&err, "ca_bundle_path");

    // And half an identity never reaches the network either.
    let mut half = provider_cfg("https://ts.example.org/fhir");
    half.client_key_path = Some(pki.client_key.clone());
    let err = TerminologyRouter::build(&external(half))
        .expect_err("half a client identity must fail the build");
    assert_cause_names(&err, "must be set together");
}

/// Assert that walking the boot failure's cause chain reaches `expected`, and
/// that the client-visible message does NOT carry it.
fn assert_cause_names(err: &SmError, expected: &str) {
    let chain = std::iter::successors(std::error::Error::source(err), |e| {
        std::error::Error::source(*e)
    })
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    assert!(
        chain.iter().any(|hop| hop.contains(expected)),
        "the cause chain must name {expected:?}, got {chain:?}"
    );
    assert!(
        !err.message.contains(expected),
        "the client-visible message must not carry the diagnosis, got {}",
        err.message
    );
}

/// The paths themselves are what the configuration carries — a sanity check
/// that the fixture material really is what the tests above claim.
#[test]
fn the_test_pki_is_well_formed() {
    assert_eq!(certs(CA_CERT_PEM).len(), 1);
    assert_eq!(certs(SERVER_CERT_PEM).len(), 1);
    assert_eq!(certs(CLIENT_CERT_PEM).len(), 1);
    let pki = Pki::write();
    for path in [&pki.ca, &pki.client_cert, &pki.client_key] {
        assert!(Path::new(path).is_file(), "{} written", path.display());
    }
}
