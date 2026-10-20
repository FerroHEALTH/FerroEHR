// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]
//! `[server.tls]` — native TLS + the client-certificate gate (the IHE ATNA
//! ITI-19 node-authentication posture) on a real listener: an HTTPS client
//! trusting the test CA reaches `/rest/status` over TLS; with
//! `client_auth = "required"` a client presenting no certificate is rejected
//! at the handshake; with `"optional"` it is served. The accept-path for a
//! *valid* client certificate is rustls' `WebPkiClientVerifier` (upstream-
//! tested); the embedded test leaf is serverAuth-only, so it cannot double as
//! a client identity here.
//!
//! Uses the same offline-generated, 100-year test CA/leaf as the ATNA syslog
//! TLS round-trip test (CN=localhost, SAN localhost/127.0.0.1), plus an EC
//! P-256 CA/leaf pair for the key-format cases and the certificate reload: a
//! pair replaced on disk is presented by the next handshake, and a broken
//! replacement leaves the pair in service untouched.

use std::io::Write;
use std::time::Duration;

use ferroehr::config::server::{ClientAuth, TlsConfig, TlsVersion};

const TEST_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIDFzCCAf+gAwIBAgIUAs65+YSFrH4uvtY9QWoTirKgkY8wDQYJKoZIhvcNAQEL
BQAwGjEYMBYGA1UEAwwPZWhyYmFzZS10ZXN0LWNhMCAXDTI2MDcwNjE3NTkwMVoY
DzIxMjYwNjEyMTc1OTAxWjAaMRgwFgYDVQQDDA9laHJiYXNlLXRlc3QtY2EwggEi
MA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQC14PGEIvdnhTn4PjCWM6yzalPu
tB924PIMTCwTLadZ6yX+RSmzQkLHQu83TR8l2D1nSfJaQMrejT6BRIZGOmhjjsWK
D2ryY+m45ALweQYfPNstbrm9CQrAoNTWzgxA21ES3dH+2J5AKxDgQq1k2KqT5Lun
NLzxpMDVUYDvx3kMgakrdhvHuSXBP9bwu3+kWrKvlhl5JndHrj6ASciPuC291knI
r/g4u7aeCbFO+XH7+PsCPNsQuEC9ONMhEokMmGbqcVAHB6uOAM+El+mp0SbKUKfd
tWBEUd3IGCVIKe0OHc4H1faoyMx9fJ+F3fJR+77P5DlHnVapRcqj6RSNLLdlAgMB
AAGjUzBRMB0GA1UdDgQWBBSx14Rc1SngmufqoAwJU9BbIE5oxzAfBgNVHSMEGDAW
gBSx14Rc1SngmufqoAwJU9BbIE5oxzAPBgNVHRMBAf8EBTADAQH/MA0GCSqGSIb3
DQEBCwUAA4IBAQCtdVYkrqn7bx0Dy2GSJencQKyH8Ih7nXLUWQdCBprCrn39zWDV
69hnAfeEuMxobUCxwsdRt06wIxadfgawB0rJWZDlEJN5ERLfvHNPE6s1G/6tdRRP
bd6B2Pv4uI7IUdfH0zrpCtjLeQOZbrU5W6fuaGtUm9GxuwjPfuYOFjnNLojWGm54
FQ6i/VNDLJThupkO95ab44mY/S12Sxvm5WnIoDZFWXdxQ8t6vnKRX+m89CmyNKP2
9TLYW57BC6+RQlUJaXgqc+1t1vsWSJeAshGQcv2BxUTTvbMVhTBPlBUVy/XcgRCf
MmtVfj2FhFGZDZxzyYU7uc0sRB9ja6qBZ9hk
-----END CERTIFICATE-----
";

const TEST_LEAF_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIDSzCCAjOgAwIBAgIUZvcUkQeuzlUI70IpDWe6esK6Q1MwDQYJKoZIhvcNAQEL
BQAwGjEYMBYGA1UEAwwPZWhyYmFzZS10ZXN0LWNhMCAXDTI2MDcwNjE3NTkwMVoY
DzIxMjYwNjEyMTc1OTAxWjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwggEiMA0GCSqG
SIb3DQEBAQUAA4IBDwAwggEKAoIBAQCTDGF0kIaNYD3CNiO9XMir1OrsU/+IBtyX
6b9zfs7mMB3NSgi7AuBNdImg3WT4EwgQ/lyX9vOVgKuyiQAWQXMl1DXauY4+z1rr
N4ob6Lc2R4OLQEh6sqyoLMVwImafOkvXJ50rowWZKPiUBw437i4s1w1iaAf16ktw
kUu+1c8mfqzcnVdPJhWR2efU1KexWxsieNLICru3h5G3kVjkc3ulou5MpHBNKY6O
WnEmdtEdwn5seJCZfep+IAs5cAv38GJ505vTEf1J9Q6cgljk/EAeB58XCHo4imx+
pfgvoXBLyWR826IyB7YqMT8lTZbAWnoU5Yuag/BgZ0zI5MBvP6PzAgMBAAGjgYww
gYkwCQYDVR0TBAIwADAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEwCwYDVR0P
BAQDAgWgMBMGA1UdJQQMMAoGCCsGAQUFBwMBMB0GA1UdDgQWBBT/CRm0D6LGZnW4
ZmpiK4+Yb53jszAfBgNVHSMEGDAWgBSx14Rc1SngmufqoAwJU9BbIE5oxzANBgkq
hkiG9w0BAQsFAAOCAQEAhMdKY/Qk5eJJhH0SsKQngd2EePo8/f3hWcucBWzCZ1zs
XTv1HoiJxPYJF9DIewF+x/TiEGow9WF8iU6EZT99w9IocIJHaFjFuFE1xmJlrlzv
5bkLIwNyrkJQzxURBzuvEwepPsF2K1RkJEqzk4d5Ffwxv6hkf+whfpXqXxwCO1PW
wQ/gmqFivCy8Zu99xO9ZYkQ/Gz5gCnna3snxutppF/xbWfQwOrq1HxgrAyIwFxAZ
CbLNvH16JKpegbWEN2llNt2kGdL62y9TPtNPr5v5prdEVnn7RUZvchABqdLYuyAW
P6DbgBmWaymqYHzOtsAx6zixYMWRkWJWMbq4mEp03Q==
-----END CERTIFICATE-----
";

const TEST_LEAF_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCTDGF0kIaNYD3C
NiO9XMir1OrsU/+IBtyX6b9zfs7mMB3NSgi7AuBNdImg3WT4EwgQ/lyX9vOVgKuy
iQAWQXMl1DXauY4+z1rrN4ob6Lc2R4OLQEh6sqyoLMVwImafOkvXJ50rowWZKPiU
Bw437i4s1w1iaAf16ktwkUu+1c8mfqzcnVdPJhWR2efU1KexWxsieNLICru3h5G3
kVjkc3ulou5MpHBNKY6OWnEmdtEdwn5seJCZfep+IAs5cAv38GJ505vTEf1J9Q6c
gljk/EAeB58XCHo4imx+pfgvoXBLyWR826IyB7YqMT8lTZbAWnoU5Yuag/BgZ0zI
5MBvP6PzAgMBAAECggEABBMyX5H2DYXXPxDdoofEybWhkojeTQmk31Kyz/wXLPSg
tPOIbSxzYoXNUB7Xc8agx13vAqZ3AK6yxJ01KRn1rxfDyV/FLTbhhzS4z4iSDG5C
TFg2oCJMXkhqar2QsinYLgpbdNGoLUo9hsyiBjzrV+Tr6n5eAEMRwL3q9g/3srTk
5NO/+mAd2ODzrz6SwWbZSQpQZ/rvY4bmCihLCl3KzXR53M4c2Q6RZOH4Y1Dk3t1X
Rk5qag+88SU3d0y7S6HZkS66psWlBGoAm13Nlp+BjUdw04pCNbtH98XEZtHIhHDu
dlG96/c9WQIalB24Do4MD7/4dpc4jlBKHRCNtcPm4QKBgQDNM7TvScNNPtOtQIfT
Zj1neBu18Ol1QgdSw1f5uSgk7dlUD4ZyARXVQE/wBhdou4Ly2K41MkkQbpMai84T
hhdgaoK8hq7uM7OeAueduEg5tZYGPOXvCi6oDxT2u0XiZmxCBR2NnPG+S9PNX6Th
dFUz2VGsYN1e8LKqFNnIVBDOKwKBgQC3c0v13xNWh3TKPcJox3PkeVhreYTQnmGk
tNVL+JWIybcBbxszkOGnKpDoK1EXLt3aY7wz/3vceTPmLLRzF119locszvVWUPeK
803JJS0BuLx2iYGbE/7Ms4uPSIE1GsnTh2R7/9q7nMcn0IDY3ccDtTVV5am5cnLu
wmnHRLNlWQKBgQDFq/W0PP5iPw9yaiNtxaOJNO6cycJbLowXcg4hhffh+y3MCFif
IeSCVT2sHnIWdeujPJA0togjyCD1BZAyxo1mV7QxIB0LYcq5gFrWWFtbHE21HJzz
NH/VbRHozGZ5veizgHIDpRhLFIin4mbWuFYLaGGBCJ63gm7Z4NQaYsD9mQKBgCkT
RcTNp/TN65aE8YyobAiSKvwVf2l+rPpGCyQxirnQAIvvprWLERtu6ncxi7yXH9GQ
V0mFQOSCtt4o0FacurAuDiI4TLKA+oxAIgCRtFwYUwpvi3d/qKOI6Ayy/Us4rkwQ
mq0xAnbTibwecVsdfTwVTNXo0HHXcGiJW0nk1g4xAoGAWRcc/XQTYPmgw3QtkyfA
z12RhYxzUUaiiGwYTbSyitIKvySud+vCPw7Mnz1cdC63XOUN/mQr2Nwht2oFtLvl
s29Z18H0uXzhfKQVu9kMRzdods9GZnHuEkPgQAO21av2ermD0646zvNsMZCAXPo1
F9DV18ILppscP5wy0SP20/U=
-----END PRIVATE KEY-----
";

/// Write the PEMs to temp files and build the `[server.tls]` section at the
/// DEFAULT protocol floor.
fn tls_files(dir: &assert_fs::TempDir, client_auth: ClientAuth) -> TlsConfig {
    tls_files_at(dir, client_auth, TlsVersion::default())
}

/// [`tls_files`] with an explicit `min_version`.
fn tls_files_at(
    dir: &assert_fs::TempDir,
    client_auth: ClientAuth,
    min_version: TlsVersion,
) -> TlsConfig {
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).expect("create pem");
        f.write_all(bytes).expect("write pem");
        path.to_string_lossy().into_owned()
    };
    TlsConfig {
        enabled: true,
        cert_file: Some(write("server.pem", TEST_LEAF_PEM)),
        key_file: Some(write("server.key", TEST_LEAF_KEY_PEM)),
        client_auth,
        client_ca_file: match client_auth {
            ClientAuth::Off => None,
            _ => Some(write("client-ca.pem", TEST_CA_PEM)),
        },
        min_version,
        reload_interval_secs: 0,
    }
}

/// Write `cert` and `key` as the server pair under `dir` (replacing any pair
/// already there) and build the `[server.tls]` section naming them.
fn pair_files(dir: &assert_fs::TempDir, cert: &[u8], key: &[u8]) -> TlsConfig {
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("write pem");
        path.to_string_lossy().into_owned()
    };
    TlsConfig {
        enabled: true,
        cert_file: Some(write("server.pem", cert)),
        key_file: Some(write("server.key", key)),
        ..TlsConfig::default()
    }
}

/// Serve a minimal router over `[server.tls]` on an ephemeral port via the
/// crate's real TLS path; return the bound port.
async fn serve_tls(tls: TlsConfig) -> u16 {
    serve_tls_reloadable(tls).await.0
}

/// [`serve_tls`], also returning the reloader the listener resolves its
/// certificate through.
async fn serve_tls_reloadable(
    tls: TlsConfig,
) -> (u16, std::sync::Arc<ferroehr_rest::tls::CertificateReloader>) {
    let app = axum::Router::new().route("/rest/status", axum::routing::get(|| async { "OK" }));
    // Bind an ephemeral port first so the test knows where to connect.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let server_tls = ferroehr_rest::tls::server_tls(&tls).expect("tls config");
    let config = server_tls.config;
    tokio::spawn(async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        axum_server::bind_rustls(
            addr,
            axum_server::tls_rustls::RustlsConfig::from_config(config),
        )
        .serve(app.into_make_service())
        .await
        .expect("tls server task");
    });
    // Wait for the listener to come up.
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return (port, server_tls.certificate);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("TLS listener did not come up");
}

/// A trust store holding ONLY the test CA: the platform verifier (reqwest's
/// default) rejects the 100-year offline test leaf.
fn test_roots() -> rustls::RootCertStore {
    use rustls::pki_types::pem::PemObject;
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls::pki_types::CertificateDer::pem_slice_iter(TEST_CA_PEM) {
        roots.add(cert.expect("ca cert")).expect("add root");
    }
    roots
}

fn https_client() -> reqwest::Client {
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_root_certificates(test_roots())
        .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

/// An HTTPS client restricted to exactly `versions` — the positive half of the
/// protocol-floor assertion (`reqwest`'s own `min_tls_version`/
/// `max_tls_version` cannot express this over the rustls backend: it has no
/// TLS 1.0/1.1 implementation to select).
fn https_client_pinned(versions: &[&'static rustls::SupportedProtocolVersion]) -> reqwest::Client {
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .expect("versions")
        .with_root_certificates(test_roots())
        .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

#[tokio::test]
async fn serves_https_with_the_configured_certificate() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let port = serve_tls(tls_files(&dir, ClientAuth::Off)).await;
    let resp = https_client()
        .get(format!("https://localhost:{port}/rest/status"))
        .send()
        .await
        .expect("https request");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn required_client_auth_rejects_certificateless_clients() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let port = serve_tls(tls_files(&dir, ClientAuth::Required)).await;
    // No client certificate → the handshake is rejected (IHE ATNA ITI-19:
    // mutually authenticated nodes; a bare TLS client never reaches HTTP).
    let outcome = https_client()
        .get(format!("https://localhost:{port}/rest/status"))
        .send()
        .await;
    assert!(
        outcome.is_err(),
        "a certificate-less client must be rejected: {outcome:?}"
    );
}

#[tokio::test]
async fn optional_client_auth_still_serves_certificateless_clients() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let port = serve_tls(tls_files(&dir, ClientAuth::Optional)).await;
    let resp = https_client()
        .get(format!("https://localhost:{port}/rest/status"))
        .send()
        .await
        .expect("https request");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
}

// ── the protocol floor ────────────────────────────────────────────────────────

/// A TLS 1.1 `ClientHello`: legacy `client_version` `0x0302` and NO
/// `supported_versions` extension, which is exactly how a pre-1.2 client
/// announces its version (RFC 4346 §7.4.1.2 — the extension is a TLS 1.3
/// construct, RFC 8446 §4.2.1).
///
/// Everything else the hello carries is deliberately acceptable, so that the
/// VERSION is the only thing left to refuse: the two offered suites are ones
/// rustls implements, and `signature_algorithms` (RFC 8446 §4.2.3) is present
/// because rustls requires that extension before it even negotiates a version
/// and would otherwise answer `handshake_failure` for a reason that has nothing
/// to do with the floor.
fn tls11_client_hello() -> Vec<u8> {
    let mut extensions: Vec<u8> = Vec::new();
    extensions.extend_from_slice(&[0x00, 0x0d, 0x00, 0x08]); // signature_algorithms, 8 bytes
    extensions.extend_from_slice(&[0x00, 0x06]); // 3 schemes
    extensions.extend_from_slice(&[0x04, 0x03]); // ecdsa_secp256r1_sha256
    extensions.extend_from_slice(&[0x08, 0x04]); // rsa_pss_rsae_sha256
    extensions.extend_from_slice(&[0x04, 0x01]); // rsa_pkcs1_sha256
    let extensions_len = u16::try_from(extensions.len()).expect("extensions length");

    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x03, 0x02]); // client_version = TLS 1.1
    body.extend_from_slice(&[0u8; 32]); // random
    body.push(0x00); // session_id: empty
    body.extend_from_slice(&[0x00, 0x04, 0xc0, 0x2f, 0xc0, 0x30]); // ECDHE_RSA AES-GCM
    body.extend_from_slice(&[0x01, 0x00]); // compression_methods: null only
    body.extend_from_slice(&extensions_len.to_be_bytes());
    body.extend_from_slice(&extensions);

    let body_len = u32::try_from(body.len()).expect("hello body length");
    let mut handshake: Vec<u8> = vec![0x01]; // client_hello
    handshake.extend_from_slice(&body_len.to_be_bytes()[1..]); // uint24
    handshake.extend_from_slice(&body);

    let handshake_len = u16::try_from(handshake.len()).expect("handshake length");
    let mut record: Vec<u8> = vec![0x16, 0x03, 0x02]; // handshake record, TLS 1.1
    record.extend_from_slice(&handshake_len.to_be_bytes());
    record.extend_from_slice(&handshake);
    record
}

/// A client offering only TLS 1.1 is refused at the handshake with a fatal
/// `protocol_version` alert, at BOTH configurable floors — RFC 8996 deprecates
/// TLS 1.0/1.1 outright, and `TlsVersion` has no variant that could select
/// them.
///
/// Asserted on the wire rather than on the config, because rustls exposes no
/// accessor for a `ServerConfig`'s enabled versions and no client that speaks
/// TLS 1.1 exists in this stack — the raw `ClientHello` above is the only way
/// to ask the question. The alert description is the one RFC 5246 §7.2.2
/// prescribes for it: "the protocol version the client has attempted to
/// negotiate is recognized but not supported".
#[tokio::test]
async fn a_pre_tls_1_2_client_is_refused_at_the_handshake() {
    for min_version in [TlsVersion::V1_3, TlsVersion::V1_2] {
        refuses_a_tls_1_1_hello(min_version).await;
    }
}

async fn refuses_a_tls_1_1_hello(min_version: TlsVersion) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = assert_fs::TempDir::new().expect("tempdir");
    let port = serve_tls(tls_files_at(&dir, ClientAuth::Off, min_version)).await;

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    socket
        .write_all(&tls11_client_hello())
        .await
        .expect("write client hello");

    let mut header = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut header))
        .await
        .expect("the server must answer the hello")
        .expect("record header");
    assert_eq!(
        header[0], 0x15,
        "min_version {min_version:?}: expected an alert record (0x15), got content type {:#04x} \
         — a 0x16 would mean the server answered a TLS 1.1 hello with a ServerHello",
        header[0]
    );

    let mut alert = [0u8; 2];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut alert))
        .await
        .expect("the alert body must follow")
        .expect("alert body");
    assert_eq!(alert[0], 0x02, "the alert must be fatal (level 2)");
    assert_eq!(
        alert[1], 70,
        "the alert must be protocol_version (70), not {}",
        alert[1]
    );
}

/// The DEFAULT floor is TLS 1.3 **only**: a client whose maximum is TLS 1.2 is
/// refused, a TLS 1.3 client is served.
///
/// This is the assertion the floor claim needs — "1.2 or better" would pass a
/// weaker configuration too. The default follows the OWASP Transport Layer
/// Security Cheat Sheet §Only Support Strong Protocols ("web applications must
/// default to TLS 1.3").
#[tokio::test]
async fn the_default_floor_serves_tls_1_3_only() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let port = serve_tls(tls_files(&dir, ClientAuth::Off)).await;

    let refused = https_client_pinned(&[&rustls::version::TLS12])
        .get(format!("https://localhost:{port}/rest/status"))
        .send()
        .await;
    assert!(
        refused.is_err(),
        "a TLS 1.2-max client must not be served at the default floor: {refused:?}"
    );

    let served = https_client_pinned(&[&rustls::version::TLS13])
        .get(format!("https://localhost:{port}/rest/status"))
        .send()
        .await
        .expect("a TLS 1.3 client must be served");
    assert_eq!(served.status(), reqwest::StatusCode::OK);
}

/// `min_version = "1.2"` is the deliberate compatibility widening: a TLS
/// 1.2-max client is now served, and TLS 1.3 still is.
///
/// Pinned so the escape hatch is known to work — a 1.2-only client is fixed by
/// naming the floor in configuration, never by turning TLS off.
#[tokio::test]
async fn the_compatibility_floor_also_serves_tls_1_2() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let port = serve_tls(tls_files_at(&dir, ClientAuth::Off, TlsVersion::V1_2)).await;

    for versions in [
        &[&rustls::version::TLS12][..],
        &[&rustls::version::TLS13][..],
    ] {
        let resp = https_client_pinned(versions)
            .get(format!("https://localhost:{port}/rest/status"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("{versions:?} client must be served at the 1.2 floor: {e}"));
        assert_eq!(resp.status(), reqwest::StatusCode::OK, "{versions:?}");
    }
}

/// Every cipher suite the server's TLS config carries is an AEAD suite with
/// ephemeral key agreement — no CBC, no RC4/3DES, no static-RSA or anonymous
/// key exchange (BCP 195 / RFC 9325 §4.2, which recommends exactly the
/// AES-GCM and ChaCha20-Poly1305 suites over ECDHE).
///
/// This is the suite half of the claim, asserted at the only seam rustls
/// exposes: the provider's suite list on the built `ServerConfig` (which the
/// enabled protocol versions then narrow — the 1.2 suites are unreachable at
/// the default 1.3-only floor). A new suite in an upstream default has to be
/// re-adjudicated here rather than arriving silently.
#[test]
fn the_configured_suites_are_aead_only_over_ephemeral_key_agreement() {
    use rustls::CipherSuite;

    let dir = assert_fs::TempDir::new().expect("tempdir");
    let config = ferroehr_rest::tls::server_tls(&tls_files(&dir, ClientAuth::Off))
        .expect("tls config")
        .config;
    let suites = &config.crypto_provider().cipher_suites;
    assert!(!suites.is_empty(), "the provider must offer suites");

    for suite in suites {
        let version = suite.version().version;
        assert!(
            matches!(
                version,
                rustls::ProtocolVersion::TLSv1_2 | rustls::ProtocolVersion::TLSv1_3
            ),
            "{:?} is offered for {version:?}, below the TLS 1.2 floor",
            suite.suite()
        );
        assert!(
            matches!(
                suite.suite(),
                CipherSuite::TLS13_AES_128_GCM_SHA256
                    | CipherSuite::TLS13_AES_256_GCM_SHA384
                    | CipherSuite::TLS13_CHACHA20_POLY1305_SHA256
                    | CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
                    | CipherSuite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
                    | CipherSuite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
                    | CipherSuite::TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256
                    | CipherSuite::TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384
                    | CipherSuite::TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256
            ),
            "{:?} is not one of the adjudicated AEAD + ephemeral-key-agreement suites",
            suite.suite()
        );
    }
}

#[tokio::test]
async fn missing_key_material_is_a_typed_boot_error() {
    let tls = TlsConfig {
        enabled: true,
        ..TlsConfig::default()
    };
    let err = ferroehr_rest::tls::server_tls(&tls).expect_err("must fail");
    assert!(err.to_string().contains("server.tls.cert_file"));
}

// ------------------------------------------------------- EC keys and reload

/// A 100-year EC P-256 test CA (named curve), generated offline with `LibreSSL`.
const TEST_EC_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIBdjCCAR2gAwIBAgIJAKiOVbI12tnRMAoGCCqGSM49BAMCMB4xHDAaBgNVBAMM
E2ZlcnJvZWhyLXRlc3QtZWMtY2EwIBcNMjYxMDE0MTg1MTI0WhgPMjEyNjA5MjAx
ODUxMjRaMB4xHDAaBgNVBAMME2ZlcnJvZWhyLXRlc3QtZWMtY2EwWTATBgcqhkjO
PQIBBggqhkjOPQMBBwNCAAR1u4YnEx2336QpA9RY3M4LQycXaKmy8be+FHUFBMDT
jb6wntl8W9Hgr1knaD7EzIwx0aOCs8iacfkauvJeETA5o0IwQDAPBgNVHRMBAf8E
BTADAQH/MA4GA1UdDwEB/wQEAwIBBjAdBgNVHQ4EFgQUkOGQfgIVWOU2fdzgM1lP
etzG67QwCgYIKoZIzj0EAwIDRwAwRAIgZESns1qCI5+B+YQwKS0eySlDeV4Fjz4p
Ksg4rl6hOYcCIHJ+sAl4j7GrhcdvjuRQxz4kgQYgu0dpunux+jocrSOL
-----END CERTIFICATE-----
";

/// The EC P-256 server leaf `TEST_EC_CA_PEM` issued (CN=localhost, SAN
/// localhost/127.0.0.1, serverAuth).
const TEST_EC_LEAF_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIBlzCCAT2gAwIBAgIJAI3GSCLsJSmPMAoGCCqGSM49BAMCMB4xHDAaBgNVBAMM
E2ZlcnJvZWhyLXRlc3QtZWMtY2EwIBcNMjYxMDE0MTg1MTI0WhgPMjEyNjA5MjAx
ODUxMjRaMBQxEjAQBgNVBAMMCWxvY2FsaG9zdDBZMBMGByqGSM49AgEGCCqGSM49
AwEHA0IABLhm2z3NfMAnLc/E1LSOKIvP8C3o1iV28QPwUENJXbPHc9SBpg1blvvx
iAj++6eSjVAHBx2Kl3jC5PgoqAp20gGjbDBqMAkGA1UdEwQCMAAwCwYDVR0PBAQD
AgeAMBMGA1UdJQQMMAoGCCsGAQUFBwMBMBoGA1UdEQQTMBGCCWxvY2FsaG9zdIcE
fwAAATAfBgNVHSMEGDAWgBSQ4ZB+AhVY5TZ93OAzWU963MbrtDAKBggqhkjOPQQD
AgNIADBFAiEAgaReGjmbq/SK8UmAHOIC9/HtUr+3Z1m+rcItIvurMJYCIEJIdwbJ
QC+qbr9pvcJ2n9QQDN1kQe8UJ1phtndF5vKu
-----END CERTIFICATE-----
";

/// The EC leaf's key as SEC1 (`openssl ecparam -genkey -noout`).
const TEST_EC_LEAF_SEC1_KEY_PEM: &[u8] = b"-----BEGIN EC PRIVATE KEY-----
MHcCAQEEIBfCoizCOqY1traVDSnFNHdf+UBJnbQORp5Pf40umVwPoAoGCCqGSM49
AwEHoUQDQgAEuGbbPc18wCctz8TUtI4oi8/wLejWJXbxA/BQQ0lds8dz1IGmDVuW
+/GICP77p5KNUAcHHYqXeMLk+CioCnbSAQ==
-----END EC PRIVATE KEY-----
";

/// The same EC leaf key as PKCS#8 (`openssl pkcs8 -topk8 -nocrypt`).
const TEST_EC_LEAF_PKCS8_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgF8KiLMI6pjW2tpUN
KcU0d1/5QEmdtA5Gnk9/jS6ZXA+hRANCAAS4Zts9zXzAJy3PxNS0jiiLz/At6NYl
dvED8FBDSV2zx3PUgaYNW5b78YgI/vunko1QBwcdipd4wuT4KKgKdtIB
-----END PRIVATE KEY-----
";

/// A self-signed X.509 v3 EC P-256 certificate whose public key carries
/// EXPLICIT curve parameters: what `LibreSSL` 3.3 `req -x509 -newkey ec -pkeyopt
/// ec_paramgen_curve:prime256v1` writes without `ec_param_enc:named_curve`.
const TEST_EXPLICIT_CURVE_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
MIICZjCCAgygAwIBAgIJAPSoLDAHYVTDMAoGCCqGSM49BAMCMBQxEjAQBgNVBAMM
CWxvY2FsaG9zdDAgFw0yNjEwMTUwNTQ1MjRaGA8yMTI2MDkyMTA1NDUyNFowFDES
MBAGA1UEAwwJbG9jYWxob3N0MIIBSzCCAQMGByqGSM49AgEwgfcCAQEwLAYHKoZI
zj0BAQIhAP////8AAAABAAAAAAAAAAAAAAAA////////////////MFsEIP////8A
AAABAAAAAAAAAAAAAAAA///////////////8BCBaxjXYqjqT57PrvVV2mIa8ZR0G
sMxTsPY7zjw+J9JgSwMVAMSdNgiG5wSTamZ44ROdJreBn36QBEEEaxfR8uEsQkf4
vOblY6RA8ncDfYEt6zOg9KE5RdiYwpZP40Li/hp/m47n60p8D54WK84zV2sxXs7L
tkBoN79R9QIhAP////8AAAAA//////////+85vqtpxeehPO5ysL8YyVRAgEBA0IA
BDa8lmLQSjc1D7ZQwWBdLY/tyYQnD3JJKDg2b5bccjzCZ/lakygnkm+QlADpnRHJ
gNDv9sdp9uGHShYNSwaNE96jUTBPMAwGA1UdEwEB/wQCMAAwDgYDVR0PAQH/BAQD
AgeAMBMGA1UdJQQMMAoGCCsGAQUFBwMBMBoGA1UdEQQTMBGCCWxvY2FsaG9zdIcE
fwAAATAKBggqhkjOPQQDAgNIADBFAiBR1BKUFwecKm7wzIVKOjxroT78yV//p90K
rzh4GSxbBwIhAKuuohK92TuOYnuv5gJK3H+XJQlQHSi83tx4TqnEl6fy
-----END CERTIFICATE-----
";

/// The key of `TEST_EXPLICIT_CURVE_PEM`, PKCS#8 with the same explicit
/// parameters.
const TEST_EXPLICIT_CURVE_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
MIIBeQIBADCCAQMGByqGSM49AgEwgfcCAQEwLAYHKoZIzj0BAQIhAP////8AAAAB
AAAAAAAAAAAAAAAA////////////////MFsEIP////8AAAABAAAAAAAAAAAAAAAA
///////////////8BCBaxjXYqjqT57PrvVV2mIa8ZR0GsMxTsPY7zjw+J9JgSwMV
AMSdNgiG5wSTamZ44ROdJreBn36QBEEEaxfR8uEsQkf4vOblY6RA8ncDfYEt6zOg
9KE5RdiYwpZP40Li/hp/m47n60p8D54WK84zV2sxXs7LtkBoN79R9QIhAP////8A
AAAA//////////+85vqtpxeehPO5ysL8YyVRAgEBBG0wawIBAQQgNwnAi5rRiOkF
UN2H95F4PHYaAt7Sz4m/7hzR332QYWehRANCAAQ2vJZi0Eo3NQ+2UMFgXS2P7cmE
Jw9ySSg4Nm+W3HI8wmf5WpMoJ5JvkJQA6Z0RyYDQ7/bHafbhh0oWDUsGjRPe
-----END PRIVATE KEY-----
";

/// The DER of the first certificate in `pem`.
fn der_of(pem: &[u8]) -> rustls::pki_types::CertificateDer<'static> {
    use rustls::pki_types::pem::PemObject;
    rustls::pki_types::CertificateDer::from_pem_slice(pem).expect("certificate pem")
}

/// A client config trusting both test CAs, so either server pair verifies.
fn client_config_trusting_both_cas() -> std::sync::Arc<rustls::ClientConfig> {
    let mut roots = test_roots();
    roots.add(der_of(TEST_EC_CA_PEM)).expect("add EC root");
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    std::sync::Arc::new(
        rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

/// Completes ONE fresh, verified handshake against the listener and returns
/// the end-entity certificate the server presented in it.
async fn presented_certificate(port: u16) -> rustls::pki_types::CertificateDer<'static> {
    tokio::task::spawn_blocking(move || {
        let name = rustls::pki_types::ServerName::try_from("localhost").expect("server name");
        let mut connection = rustls::ClientConnection::new(client_config_trusting_both_cas(), name)
            .expect("client connection");
        let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout");
        while connection.is_handshaking() {
            connection
                .complete_io(&mut socket)
                .expect("the handshake must verify against the test CAs");
        }
        connection
            .peer_certificates()
            .and_then(|chain| chain.first())
            .expect("the server must present a certificate")
            .clone()
            .into_owned()
    })
    .await
    .expect("handshake task")
}

/// An EC P-256 key loads in PKCS#8 and in SEC1 alike, and the listener serves
/// a verified handshake with it.
#[tokio::test]
async fn an_ec_p256_key_loads_as_pkcs8_and_as_sec1() {
    for key in [TEST_EC_LEAF_PKCS8_KEY_PEM, TEST_EC_LEAF_SEC1_KEY_PEM] {
        let dir = assert_fs::TempDir::new().expect("tempdir");
        let port = serve_tls(pair_files(&dir, TEST_EC_LEAF_PEM, key)).await;
        assert_eq!(
            presented_certificate(port).await,
            der_of(TEST_EC_LEAF_PEM),
            "{}",
            String::from_utf8_lossy(key)
        );
    }
}

/// An EC certificate whose public key carries explicit curve parameters never
/// matches its key byte for byte, and the refusal says so and names the fix,
/// rather than a bare mismatch (RFC 5480 §2.1.1: `namedCurve` only).
#[test]
fn an_explicit_curve_certificate_is_refused_with_the_named_curve_remedy() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = pair_files(&dir, TEST_EXPLICIT_CURVE_PEM, TEST_EXPLICIT_CURVE_KEY_PEM);
    let err = ferroehr_rest::tls::server_tls(&tls).expect_err("must refuse");
    assert!(
        matches!(err, ferroehr_rest::tls::TlsError::EcKeyMismatch { .. }),
        "{err:?}"
    );
    let text = err.to_string();
    assert!(text.contains("RFC 5480"), "{text}");
    assert!(text.contains("ec_param_enc:named_curve"), "{text}");
}

/// A key that belongs to a different certificate is a typed mismatch.
#[test]
fn a_key_of_another_certificate_is_a_key_mismatch() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = pair_files(&dir, TEST_EC_LEAF_PEM, TEST_LEAF_KEY_PEM);
    let err = ferroehr_rest::tls::server_tls(&tls).expect_err("must refuse");
    assert!(
        matches!(err, ferroehr_rest::tls::TlsError::KeyMismatch { .. }),
        "{err:?}"
    );
}

/// A renewed pair on disk is presented by the next handshake without a
/// restart; a broken replacement is refused, reported once, and the pair in
/// service stays in service.
#[tokio::test]
async fn a_renewed_certificate_is_served_without_a_restart_and_a_broken_one_is_not() {
    use ferroehr_rest::tls::Reloaded;

    let dir = assert_fs::TempDir::new().expect("tempdir");
    let (port, reloader) =
        serve_tls_reloadable(pair_files(&dir, TEST_LEAF_PEM, TEST_LEAF_KEY_PEM)).await;
    assert_eq!(presented_certificate(port).await, der_of(TEST_LEAF_PEM));
    assert_eq!(
        reloader.reload().await.expect("unchanged files"),
        Reloaded::Unchanged
    );

    pair_files(&dir, TEST_EC_LEAF_PEM, TEST_EC_LEAF_PKCS8_KEY_PEM);
    assert_eq!(
        reloader.reload().await.expect("the renewed pair loads"),
        Reloaded::Replaced
    );
    assert_eq!(presented_certificate(port).await, der_of(TEST_EC_LEAF_PEM));

    // A replacement certificate without its matching key.
    pair_files(&dir, TEST_LEAF_PEM, TEST_EC_LEAF_PKCS8_KEY_PEM);
    let refused = reloader.reload().await.expect_err("a mismatched pair");
    assert!(
        matches!(refused, ferroehr_rest::tls::TlsError::EcKeyMismatch { .. }),
        "{refused:?}"
    );
    assert_eq!(presented_certificate(port).await, der_of(TEST_EC_LEAF_PEM));
    assert_eq!(
        reloader
            .reload()
            .await
            .expect("the refused pair is not re-reported"),
        Reloaded::Unchanged
    );

    // A key file that is not PEM at all.
    pair_files(&dir, TEST_LEAF_PEM, b"not a key");
    reloader.reload().await.expect_err("an unreadable key");
    assert_eq!(presented_certificate(port).await, der_of(TEST_EC_LEAF_PEM));
}

/// The watcher the listener runs picks a renewal up on its own interval.
#[tokio::test]
async fn the_watcher_serves_a_renewal_on_its_interval() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let (port, reloader) =
        serve_tls_reloadable(pair_files(&dir, TEST_LEAF_PEM, TEST_LEAF_KEY_PEM)).await;
    let watcher = tokio::spawn(reloader.watch(Duration::from_millis(50)));

    pair_files(&dir, TEST_EC_LEAF_PEM, TEST_EC_LEAF_SEC1_KEY_PEM);
    let renewed = der_of(TEST_EC_LEAF_PEM);
    let mut presented = presented_certificate(port).await;
    for _ in 0..100 {
        if presented == renewed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        presented = presented_certificate(port).await;
    }
    watcher.abort();
    assert_eq!(presented, renewed, "the watcher never served the renewal");
}

// ── the client CA reload (#3769) ─────────────────────────────────────────────

/// A second client CA (`CN=ehrbase-terminology-test-ca`), the one the
/// terminology mTLS suite signs its client certificate with.
const ROTATED_CLIENT_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
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

/// A client leaf (`CN=ehrbase-cdr`, `extendedKeyUsage=clientAuth`) signed by
/// [`ROTATED_CLIENT_CA_PEM`].
const ROTATED_CLIENT_CERT_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
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

/// The key of [`ROTATED_CLIENT_CERT_PEM`].
const ROTATED_CLIENT_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----
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

/// One handshake presenting the rotated client certificate, then `GET
/// /rest/status`; `true` when the server answered.
async fn served_with_client_certificate(port: u16) -> bool {
    use rustls::pki_types::pem::PemObject;
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let chain = vec![der_of(ROTATED_CLIENT_CERT_PEM)];
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(ROTATED_CLIENT_KEY_PEM)
        .expect("client key");
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_root_certificates(test_roots())
        .with_client_auth_cert(chain, key)
        .expect("client identity");
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");
    client
        .get(format!("https://localhost:{port}/rest/status"))
        .send()
        .await
        .is_ok_and(|r| r.status() == reqwest::StatusCode::OK)
}

/// Serve a minimal router through `config` on an ephemeral port, keeping the
/// caller's handle on the configuration so it can swap one in.
async fn serve_config(config: axum_server::tls_rustls::RustlsConfig) -> u16 {
    let app = axum::Router::new().route("/rest/status", axum::routing::get(|| async { "OK" }));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    tokio::spawn(async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        axum_server::bind_rustls(addr, config)
            .serve(app.into_make_service())
            .await
            .expect("tls server task");
    });
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return port;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("TLS listener did not come up");
}

/// A client CA rotated on disk verifies the next handshake once the reloader
/// has read it and the listener has swapped the configuration in; a broken
/// replacement is refused and the rotated CA stays in force.
#[tokio::test]
async fn a_rotated_client_ca_verifies_without_a_restart() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = tls_files(&dir, ClientAuth::Required);
    let server_tls = ferroehr_rest::tls::server_tls(&tls).expect("tls config");
    let listener_config = axum_server::tls_rustls::RustlsConfig::from_config(server_tls.config);
    let port = serve_config(listener_config.clone()).await;
    let reloader = server_tls
        .client_ca
        .expect("a client CA reloader under client_auth");

    assert!(
        !served_with_client_certificate(port).await,
        "a certificate of another CA is refused"
    );

    let ca_path = tls.client_ca_file.clone().expect("client_ca_file");
    std::fs::write(&ca_path, ROTATED_CLIENT_CA_PEM).expect("rotate the client CA");
    let config = reloader
        .reload()
        .await
        .expect("the rotated CA loads")
        .expect("a changed CA builds a configuration");
    listener_config.reload_from_config(config);
    assert!(
        served_with_client_certificate(port).await,
        "the rotated CA verifies the next handshake"
    );
    assert!(
        reloader.reload().await.expect("unchanged").is_none(),
        "an unchanged file builds nothing"
    );

    std::fs::write(&ca_path, b"not a certificate").expect("break the client CA");
    reloader.reload().await.expect_err("a broken CA is refused");
    assert!(
        reloader.reload().await.expect("reported once").is_none(),
        "the refused file is not re-reported"
    );
    assert!(
        served_with_client_certificate(port).await,
        "the rotated CA stays in force"
    );
}
