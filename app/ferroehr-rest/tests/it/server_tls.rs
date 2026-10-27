// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]
//! `[server.tls]` — native TLS + the client-certificate gate (the IHE ATNA
//! ITI-19 node-authentication posture) on a real listener served by the family
//! listener (`ferrohealth_server::serve::run`) as FerroEHR runs it: an HTTPS client
//! trusting the test CA reaches `/rest/status` over TLS; with
//! `client_auth = "required"` a client presenting no certificate is rejected
//! at the handshake; with `"optional"` it is served. The accept-path for a
//! *valid* client certificate is rustls' `WebPkiClientVerifier` (upstream-
//! tested); the test leaf is serverAuth-only, so it cannot double as a client
//! identity here.
//!
//! Every certificate and key is minted at run time with `rcgen` (`Pki`): an
//! RSA CA and leaf (CN=localhost, SAN localhost/127.0.0.1), plus an EC P-256
//! CA and leaf for the key-format cases and the certificate reload: a pair
//! replaced on disk is presented once `reload_interval_s` passes, and a broken
//! replacement leaves the pair in service untouched.

use std::future::Future;
use std::io::Write;
use std::time::Duration;

use ferrohealth_health::lifecycle::Lifecycle;
use ferrohealth_server::section::{ClientAuth, ServerSection, TlsSection, TlsVersion};

/// How many days past today the minted certificates stay valid: short, so a
/// platform verifier (macOS caps a TLS server certificate at 825 days) never
/// refuses one.
const VALIDITY_DAYS: i64 = 30;

/// The certificates and keys these tests serve and trust, minted once per
/// test process with `rcgen` and valid from yesterday to [`VALIDITY_DAYS`]
/// from today, so nothing is committed and nothing expires under the tests.
struct Pki {
    /// An RSA CA (`CN=ferroehr-test-ca`), also the client CA of the gate.
    ca: String,
    /// The RSA server leaf it issued (CN=localhost, SAN localhost/127.0.0.1,
    /// serverAuth only, so it cannot double as a client identity).
    leaf: String,
    /// The RSA leaf's key, PKCS#8.
    leaf_key: String,
    /// An EC P-256 CA (named curve).
    ec_ca: String,
    /// The EC P-256 server leaf `ec_ca` issued (CN=localhost, serverAuth).
    ec_leaf: String,
    /// The EC leaf's key as PKCS#8.
    ec_leaf_pkcs8: String,
    /// The same EC leaf key as SEC1 (`EC PRIVATE KEY`).
    ec_leaf_sec1: String,
    /// An EC P-256 certificate whose public key carries EXPLICIT curve
    /// parameters, what `openssl req -newkey ec` writes without
    /// `ec_param_enc:named_curve`.
    explicit_curve: String,
    /// An EC P-256 key for `explicit_curve`, PKCS#8.
    explicit_curve_key: String,
    /// A second client CA, the one a rotation swaps in.
    rotated_ca: String,
    /// A client leaf (`extendedKeyUsage=clientAuth`) `rotated_ca` issued.
    rotated_client: String,
    /// The rotated client leaf's key, PKCS#8.
    rotated_client_key: String,
}

static PKI: std::sync::LazyLock<Pki> = std::sync::LazyLock::new(Pki::mint);

impl Pki {
    fn mint() -> Self {
        use rcgen::{KeyPair, PKCS_ECDSA_P256_SHA256, PKCS_RSA_SHA256};
        let ca_key = KeyPair::generate_for(&PKCS_RSA_SHA256).expect("RSA CA key");
        let ca = issuer("ferroehr-test-ca", ca_key);
        let leaf_key = KeyPair::generate_for(&PKCS_RSA_SHA256).expect("RSA leaf key");
        let leaf = leaf_cert(&leaf_key, rcgen::ExtendedKeyUsagePurpose::ServerAuth, &ca);

        let ec_ca = issuer(
            "ferroehr-test-ec-ca",
            KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("EC CA key"),
        );
        let ec_leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("EC leaf key");
        let ec_leaf = leaf_cert(
            &ec_leaf_key,
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            &ec_ca,
        );

        let explicit_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("EC key");
        let named = leaf_cert(
            &explicit_key,
            rcgen::ExtendedKeyUsagePurpose::ServerAuth,
            &ec_ca,
        );

        let rotated_ca = issuer(
            "ferroehr-rotated-client-ca",
            KeyPair::generate_for(&PKCS_RSA_SHA256).expect("rotated CA key"),
        );
        let client_key = KeyPair::generate_for(&PKCS_RSA_SHA256).expect("client key");
        let client = leaf_cert(
            &client_key,
            rcgen::ExtendedKeyUsagePurpose::ClientAuth,
            &rotated_ca,
        );

        Self {
            ca: ca.certificate.pem(),
            leaf: leaf.pem(),
            leaf_key: leaf_key.serialize_pem(),
            ec_ca: ec_ca.certificate.pem(),
            ec_leaf: ec_leaf.pem(),
            ec_leaf_pkcs8: ec_leaf_key.serialize_pem(),
            ec_leaf_sec1: pem_block("EC PRIVATE KEY", &sec1_of(&ec_leaf_key.serialize_der())),
            explicit_curve: pem_block("CERTIFICATE", &with_explicit_curve(named.der())),
            explicit_curve_key: explicit_key.serialize_pem(),
            rotated_ca: rotated_ca.certificate.pem(),
            rotated_client: client.pem(),
            rotated_client_key: client_key.serialize_pem(),
        }
    }
}

/// A CA with its certificate, ready to issue.
struct Authority {
    certificate: rcgen::Certificate,
    issuer: rcgen::Issuer<'static, rcgen::KeyPair>,
}

/// Sets `params` valid from yesterday to [`VALIDITY_DAYS`] from today, in UTC.
fn stamp_validity(params: &mut rcgen::CertificateParams) {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let stamp = |date: jiff::civil::Date| {
        rcgen::date_time_ymd(
            i32::from(date.year()),
            u8::try_from(date.month()).expect("month"),
            u8::try_from(date.day()).expect("day"),
        )
    };
    let last = today
        .checked_add(jiff::Span::new().days(VALIDITY_DAYS))
        .expect("end date");
    params.not_before = stamp(today.yesterday().expect("yesterday"));
    params.not_after = stamp(last);
}

/// A self-signed CA named `name` over `key`.
fn issuer(name: &str, key: rcgen::KeyPair) -> Authority {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyUsagePurpose};
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA params");
    params.distinguished_name.push(DnType::CommonName, name);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    stamp_validity(&mut params);
    let certificate = params.self_signed(&key).expect("CA certificate");
    Authority {
        certificate,
        issuer: rcgen::Issuer::new(params, key),
    }
}

/// A leaf for localhost and 127.0.0.1 over `key`, for `purpose`, issued by `ca`.
fn leaf_cert(
    key: &rcgen::KeyPair,
    purpose: rcgen::ExtendedKeyUsagePurpose,
    ca: &Authority,
) -> rcgen::Certificate {
    use rcgen::{CertificateParams, DnType, KeyUsagePurpose};
    let mut params = CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
        .expect("leaf params");
    params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![purpose];
    stamp_validity(&mut params);
    params.signed_by(key, &ca.issuer).expect("leaf certificate")
}

/// `der` as a PEM block labelled `label`.
fn pem_block(label: &str, der: &[u8]) -> String {
    pem::encode(&pem::Pem::new(label, der.to_vec()))
}

/// The DER of the element `reader` is at, whatever its type.
fn raw_element(reader: yasna::BERReader<'_, '_>) -> yasna::ASN1Result<Vec<u8>> {
    reader.read_der()
}

/// The `ECPrivateKey` (RFC 5915 §3, the SEC1 form) a PKCS#8 EC key wraps:
/// the `privateKey` OCTET STRING of `PrivateKeyInfo` (RFC 5208 §5).
fn sec1_of(pkcs8: &[u8]) -> Vec<u8> {
    yasna::parse_der(pkcs8, |reader| {
        reader.read_sequence(|reader| {
            reader.next().read_u8()?;
            reader.next().read_der()?;
            let key = reader.next().read_bytes()?;
            reader.read_optional(raw_element)?;
            Ok(key)
        })
    })
    .expect("a PKCS#8 EC key")
}

/// `certificate` with its subject public key's `namedCurve` parameter
/// replaced by the explicit P-256 `ECParameters` (RFC 3279 §2.3.5), the
/// form RFC 5480 §2.1.1 forbids; the signature no longer verifies, which the
/// loader never checks.
fn with_explicit_curve(certificate: &[u8]) -> Vec<u8> {
    let (tbs, algorithm, signature) = yasna::parse_der(certificate, |reader| {
        reader.read_sequence(|reader| {
            let tbs = reader.next().collect_sequence_of(raw_element)?;
            let algorithm = reader.next().read_der()?;
            let signature = reader.next().read_bitvec_bytes()?;
            Ok((tbs, algorithm, signature))
        })
    })
    .expect("a DER certificate");
    let tbs: Vec<Vec<u8>> = tbs
        .into_iter()
        .map(|field| explicit_spki(&field).unwrap_or(field))
        .collect();
    yasna::construct_der(|writer| {
        writer.write_sequence(|writer| {
            writer.next().write_sequence(|writer| {
                for field in &tbs {
                    writer.next().write_der(field);
                }
            });
            writer.next().write_der(&algorithm);
            writer.next().write_bitvec_bytes(&signature.0, signature.1);
        });
    })
}

/// `id-ecPublicKey` (RFC 5480 §2.1.1).
const EC_PUBLIC_KEY: &[u64] = &[1, 2, 840, 10045, 2, 1];
/// `secp256r1`, the P-256 named curve (RFC 5480 §2.1.1.1).
const SECP256R1: &[u64] = &[1, 2, 840, 10045, 3, 1, 7];
/// `prime-field` (RFC 3279 §2.3.5).
const PRIME_FIELD: &[u64] = &[1, 2, 840, 10045, 1, 1];
/// The P-256 field prime (SEC 2 v2 §2.4.2), hex.
const P256_P: &str = "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff";
/// The P-256 coefficient `a`.
const P256_A: &str = "ffffffff00000001000000000000000000000000fffffffffffffffffffffffc";
/// The P-256 coefficient `b`.
const P256_B: &str = "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b";
/// The P-256 base point, uncompressed.
const P256_G: &str = "046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5";
/// The P-256 base point's order.
const P256_N: &str = "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551";

/// The bytes a hex string spells.
fn unhex(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16).expect("hex")
        })
        .collect()
}

/// `field` rewritten with explicit P-256 parameters when it is a P-256
/// `SubjectPublicKeyInfo` with the named curve; `None` for any other field.
fn explicit_spki(field: &[u8]) -> Option<Vec<u8>> {
    let named = yasna::models::ObjectIdentifier::from_slice(SECP256R1);
    let ec = yasna::models::ObjectIdentifier::from_slice(EC_PUBLIC_KEY);
    // NOTE: a TBS field of another shape is not the key, so its parse failure is the answer.
    let point = yasna::parse_der(field, |reader| {
        reader.read_sequence(|reader| {
            let (algorithm, curve) = reader.next().read_sequence(|reader| {
                Ok((reader.next().read_oid()?, reader.next().read_oid()?))
            })?;
            let point = reader.next().read_bitvec_bytes()?;
            Ok((algorithm == ec && curve == named).then_some(point))
        })
    })
    .ok()
    .flatten()?;
    Some(yasna::construct_der(|writer| {
        writer.write_sequence(|writer| {
            writer.next().write_sequence(|writer| {
                writer.next().write_oid(&ec);
                writer.next().write_sequence(|writer| {
                    writer.next().write_u8(1);
                    writer.next().write_sequence(|writer| {
                        writer
                            .next()
                            .write_oid(&yasna::models::ObjectIdentifier::from_slice(PRIME_FIELD));
                        writer.next().write_bigint_bytes(&unhex(P256_P), true);
                    });
                    writer.next().write_sequence(|writer| {
                        writer.next().write_bytes(&unhex(P256_A));
                        writer.next().write_bytes(&unhex(P256_B));
                    });
                    writer.next().write_bytes(&unhex(P256_G));
                    writer.next().write_bigint_bytes(&unhex(P256_N), true);
                    writer.next().write_u8(1);
                });
            });
            writer.next().write_bitvec_bytes(&point.0, point.1);
        });
    }))
}

/// Write the PEMs to temp files and build the `[server.tls]` section at the
/// DEFAULT protocol floor.
fn tls_files(dir: &assert_fs::TempDir, client_auth: ClientAuth) -> TlsSection {
    tls_files_at(dir, client_auth, TlsVersion::default())
}

/// [`tls_files`] with an explicit `min_version`.
fn tls_files_at(
    dir: &assert_fs::TempDir,
    client_auth: ClientAuth,
    min_version: TlsVersion,
) -> TlsSection {
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).expect("create pem");
        f.write_all(bytes).expect("write pem");
        path
    };
    TlsSection {
        client_auth,
        client_ca_file: match client_auth {
            ClientAuth::None => None,
            _ => Some(write("client-ca.pem", PKI.ca.as_bytes())),
        },
        min_version,
        ..TlsSection::new(
            write("server.pem", PKI.leaf.as_bytes()),
            write("server.key", PKI.leaf_key.as_bytes()),
        )
    }
}

/// Write `cert` and `key` as the server pair under `dir` (replacing any pair
/// already there) and build the `[server.tls]` section naming them.
fn pair_files(dir: &assert_fs::TempDir, cert: &[u8], key: &[u8]) -> TlsSection {
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("write pem");
        path
    };
    TlsSection::new(write("server.pem", cert), write("server.key", key))
}

/// Serve a minimal router over `[server.tls]` on an ephemeral port through the
/// family listener FerroEHR serves on; return the bound port.
async fn serve_tls(tls: TlsSection) -> u16 {
    let app = axum::Router::new().route("/rest/status", axum::routing::get(|| async { "OK" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    // Fail on the test's own thread when the files do not load, rather than in
    // the listener task.
    ferrohealth_server::tls::server_config(&tls).expect("tls config");
    let section = ServerSection {
        tls: Some(tls),
        ..ServerSection::default()
    };
    tokio::spawn(async move {
        ferrohealth_server::serve::run(
            listener,
            app,
            &section,
            Lifecycle::default(),
            std::future::pending(),
        )
        .await
        .expect("tls server task");
    });
    port
}

/// A trust store holding ONLY the test CA, which no platform store holds.
fn test_roots() -> rustls::RootCertStore {
    use rustls::pki_types::pem::PemObject;
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls::pki_types::CertificateDer::pem_slice_iter(PKI.ca.as_bytes()) {
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
    let port = serve_tls(tls_files(&dir, ClientAuth::None)).await;
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
    for min_version in [TlsVersion::Tls13, TlsVersion::Tls12] {
        refuses_a_tls_1_1_hello(min_version).await;
    }
}

async fn refuses_a_tls_1_1_hello(min_version: TlsVersion) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = assert_fs::TempDir::new().expect("tempdir");
    let port = serve_tls(tls_files_at(&dir, ClientAuth::None, min_version)).await;

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
    let port = serve_tls(tls_files(&dir, ClientAuth::None)).await;

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
    let port = serve_tls(tls_files_at(&dir, ClientAuth::None, TlsVersion::Tls12)).await;

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
    let config = ferrohealth_server::tls::server_config(&tls_files(&dir, ClientAuth::None))
        .expect("tls config");
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
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = TlsSection::new(dir.path().join("absent.pem"), dir.path().join("absent.key"));
    let err = ferrohealth_server::tls::server_config(&tls).expect_err("must fail");
    assert!(
        matches!(err, ferrohealth_server::tls::TlsError::Read { .. }),
        "{err:?}"
    );
    assert!(
        err.to_string().contains("server.tls.certificate_file"),
        "{err}"
    );
}

// ------------------------------------------------------- EC keys and reload

/// The DER of the first certificate in `pem`.
fn der_of(pem: &[u8]) -> rustls::pki_types::CertificateDer<'static> {
    use rustls::pki_types::pem::PemObject;
    rustls::pki_types::CertificateDer::from_pem_slice(pem).expect("certificate pem")
}

/// A client config trusting both test CAs, so either server pair verifies.
fn client_config_trusting_both_cas() -> std::sync::Arc<rustls::ClientConfig> {
    let mut roots = test_roots();
    roots
        .add(der_of(PKI.ec_ca.as_bytes()))
        .expect("add EC root");
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
    for key in [PKI.ec_leaf_pkcs8.as_bytes(), PKI.ec_leaf_sec1.as_bytes()] {
        let dir = assert_fs::TempDir::new().expect("tempdir");
        let port = serve_tls(pair_files(&dir, PKI.ec_leaf.as_bytes(), key)).await;
        assert_eq!(
            presented_certificate(port).await,
            der_of(PKI.ec_leaf.as_bytes()),
            "{}",
            String::from_utf8_lossy(key)
        );
    }
}

/// An EC certificate whose public key carries explicit curve parameters never
/// matches its key byte for byte, so the pair is refused at load (RFC 5480
/// §2.1.1: `namedCurve` only).
#[test]
fn an_explicit_curve_certificate_is_refused() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = pair_files(
        &dir,
        PKI.explicit_curve.as_bytes(),
        PKI.explicit_curve_key.as_bytes(),
    );
    let err = ferrohealth_server::tls::server_config(&tls).expect_err("must refuse");
    assert!(
        matches!(err, ferrohealth_server::tls::TlsError::Config(_)),
        "{err:?}"
    );
}

/// A key that belongs to a different certificate is refused at load.
#[test]
fn a_key_of_another_certificate_is_refused() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = pair_files(&dir, PKI.ec_leaf.as_bytes(), PKI.leaf_key.as_bytes());
    let err = ferrohealth_server::tls::server_config(&tls).expect_err("must refuse");
    assert!(
        matches!(err, ferrohealth_server::tls::TlsError::Config(_)),
        "{err:?}"
    );
}

/// Polls `check` every 100 ms for up to ten seconds, the bound a one-second
/// reload interval comfortably meets; `true` once it held.
async fn eventually<F, Fut>(mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for _ in 0..100 {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// A renewed pair on disk is presented once the reload interval passes, with
/// no restart; a broken replacement is refused and the pair in service stays
/// in service.
#[tokio::test]
async fn a_renewed_certificate_is_served_without_a_restart_and_a_broken_one_is_not() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = TlsSection {
        reload_interval_s: std::num::NonZeroU64::new(1),
        ..pair_files(&dir, PKI.leaf.as_bytes(), PKI.leaf_key.as_bytes())
    };
    let port = serve_tls(tls).await;
    assert_eq!(
        presented_certificate(port).await,
        der_of(PKI.leaf.as_bytes())
    );

    pair_files(&dir, PKI.ec_leaf.as_bytes(), PKI.ec_leaf_pkcs8.as_bytes());
    let renewed = der_of(PKI.ec_leaf.as_bytes());
    assert!(
        eventually(|| async { presented_certificate(port).await == renewed }).await,
        "the renewed pair was never served"
    );

    // A replacement certificate without its matching key, then a key file that
    // is not PEM at all: each read is refused and the renewed pair stays.
    for (cert, key) in [
        (PKI.leaf.as_bytes(), PKI.ec_leaf_pkcs8.as_bytes()),
        (PKI.leaf.as_bytes(), &b"not a key"[..]),
    ] {
        pair_files(&dir, cert, key);
        tokio::time::sleep(Duration::from_millis(2_500)).await;
        assert_eq!(presented_certificate(port).await, renewed);
    }
}

/// A broken replacement read on demand is refused and the running
/// configuration stays the one the listener hands out.
#[test]
fn a_reload_that_does_not_read_keeps_the_running_configuration() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = pair_files(&dir, PKI.leaf.as_bytes(), PKI.leaf_key.as_bytes());
    let reloadable = ferrohealth_server::tls::ReloadableTls::new(&tls).expect("the pair loads");
    let running = reloadable.current();
    pair_files(&dir, PKI.leaf.as_bytes(), PKI.ec_leaf_pkcs8.as_bytes());
    reloadable
        .reload()
        .expect_err("a mismatched pair is refused");
    assert!(std::sync::Arc::ptr_eq(&running, &reloadable.current()));
    pair_files(&dir, PKI.ec_leaf.as_bytes(), PKI.ec_leaf_sec1.as_bytes());
    reloadable.reload().expect("the renewed pair loads");
    assert!(!std::sync::Arc::ptr_eq(&running, &reloadable.current()));
}

// ── the client CA reload (#3769) ─────────────────────────────────────────────

/// One handshake presenting the rotated client certificate, then `GET
/// /rest/status`; `true` when the server answered.
async fn served_with_client_certificate(port: u16) -> bool {
    use rustls::pki_types::pem::PemObject;
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let chain = vec![der_of(PKI.rotated_client.as_bytes())];
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(PKI.rotated_client_key.as_bytes())
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

/// A client CA rotated on disk verifies the handshakes after the reload
/// interval, with no restart; a broken replacement is refused and the rotated
/// CA stays in force.
#[tokio::test]
async fn a_rotated_client_ca_verifies_without_a_restart() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let tls = TlsSection {
        reload_interval_s: std::num::NonZeroU64::new(1),
        ..tls_files(&dir, ClientAuth::Required)
    };
    let ca_path = tls.client_ca_file.clone().expect("client_ca_file");
    let port = serve_tls(tls).await;

    assert!(
        !served_with_client_certificate(port).await,
        "a certificate of another CA is refused"
    );

    std::fs::write(&ca_path, PKI.rotated_ca.as_bytes()).expect("rotate the client CA");
    assert!(
        eventually(|| served_with_client_certificate(port)).await,
        "the rotated CA verifies a handshake once the files are read again"
    );

    std::fs::write(&ca_path, b"not a certificate").expect("break the client CA");
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(
        served_with_client_certificate(port).await,
        "the rotated CA stays in force"
    );
}
