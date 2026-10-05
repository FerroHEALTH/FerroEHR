// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RFC 5425 TLS framing round-trip (§8.5): send an octet-counted syslog record
//! over a real TLS connection to an in-process rustls listener that trusts a
//! generated test CA, and assert the exact framed bytes arrive.
//!
//! The test PKI below was generated offline with `openssl` — no `rcgen` needed:
//! a CA (`CN=ferroehr-test-ca`) and a leaf (`CN=localhost`, SAN
//! `localhost`/`127.0.0.1`, `serverAuth`) it signed. The client trusts the CA;
//! the server presents the leaf.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::Duration;

use ferroehr::system_log::syslog::{
    TlsTransport, Transport, add_roots, assemble_syslog, frame_octet_counting,
};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

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

fn server_config() -> Arc<rustls::ServerConfig> {
    use rustls::pki_types::pem::PemObject;
    let certs: Vec<_> = rustls::pki_types::CertificateDer::pem_slice_iter(TEST_LEAF_PEM)
        .collect::<Result<_, _>>()
        .expect("certs");
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(TEST_LEAF_KEY_PEM).expect("key");

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("server cert");
    Arc::new(config)
}

fn client_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    add_roots(&mut roots, TEST_CA_PEM).expect("add roots");
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    Arc::new(config)
}

#[tokio::test]
async fn tls_octet_counted_frame_round_trips() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let acceptor = TlsAcceptor::from(server_config());

    // Server: accept one connection, read the framed record.
    let server = tokio::spawn(async move {
        let (tcp, _peer) = listener.accept().await.expect("accept");
        let mut tls = acceptor.accept(tcp).await.expect("tls accept");
        // Read until we have the whole octet-counted frame (or the peer closes).
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            // EOF and a read error both end the read the same way: whatever
            // arrived so far is the frame.
            match tls.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    // A single small write arrives as one TLS record.
                    if buf.len() >= 8 {
                        break;
                    }
                }
            }
        }
        buf
    });

    // Client: send one assembled + octet-counted syslog record.
    let ts = "2026-07-06T12:00:00Z".parse().unwrap();
    let syslog = assemble_syslog("cdr-01", "ferroehr", &ts, "<AuditMessage/>");
    let expected = frame_octet_counting(&syslog);

    let tls = TlsTransport::new(client_config(), "localhost", port).expect("client");
    let mut transport = Transport::Tls(Box::new(tls));
    transport.send(&syslog).await.expect("tls send");

    let received = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("server timeout")
        .expect("server join");

    // The received bytes begin with the octet-count framing "<len> <SYSLOG-MSG>".
    assert!(
        received.starts_with(&expected[..expected.len().min(received.len())]),
        "framed record mismatch: got {:?}",
        String::from_utf8_lossy(&received)
    );
    let text = String::from_utf8_lossy(&received);
    assert!(
        text.contains(&format!("{} ", syslog.len())),
        "octet count prefix present: {text}"
    );
    assert!(text.contains("<AuditMessage/>"), "payload present: {text}");
}
