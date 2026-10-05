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
//! TLS round-trip test (CN=localhost, SAN localhost/127.0.0.1).

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
    }
}

/// Serve a minimal router over `[server.tls]` on an ephemeral port via the
/// crate's real TLS path; return the bound port.
async fn serve_tls(tls: TlsConfig) -> u16 {
    let app = axum::Router::new().route("/rest/status", axum::routing::get(|| async { "OK" }));
    // Bind an ephemeral port first so the test knows where to connect.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let config = ferroehr_rest::tls_server_config(&tls).expect("tls config");
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
            return port;
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
    let config =
        ferroehr_rest::tls_server_config(&tls_files(&dir, ClientAuth::Off)).expect("tls config");
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
    let err = ferroehr_rest::tls_server_config(&tls).expect_err("must fail");
    assert!(err.to_string().contains("server.tls.cert_file"));
}
