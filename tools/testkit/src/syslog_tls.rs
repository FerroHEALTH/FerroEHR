// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An in-process syslog-over-TLS Audit Record Repository for the audit tests:
//! a rustls listener that presents a test leaf its test CA signed, and hands
//! over every RFC 5425 §4.3 frame it reads.
//!
//! The test PKI was generated offline with `openssl`: a CA
//! (`CN=ehrbase-test-ca`) and a leaf (`CN=localhost`, SAN `localhost` and
//! `127.0.0.1`, `serverAuth`) it signed. No openEHR spec governs test
//! infrastructure: our own design/extension.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;

/// The test CA's certificate, PEM.
pub const TEST_CA_PEM: &[u8] = b"-----BEGIN CERTIFICATE-----
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

/// The leaf the listener presents, PEM.
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

/// The leaf's private key, PEM.
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

/// Why the listener could not start.
#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    /// The test PKI did not read as PEM, or rustls refused it.
    #[error("the test PKI is unusable")]
    Pki(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The listener could not bind, or the CA file could not be written.
    #[error("the listener could not start")]
    Io(#[from] std::io::Error),
}

/// A running listener on `127.0.0.1`.
#[derive(Debug)]
pub struct SyslogTlsListener {
    /// The port it listens on.
    pub port: u16,
    /// A file holding [`TEST_CA_PEM`], for `[audit.syslog] tls_ca_file`.
    pub ca_file: PathBuf,
    frames: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl SyslogTlsListener {
    /// Binds a listener on a free port and starts accepting connections.
    ///
    /// # Errors
    /// [`ListenerError`] when the PKI is unusable, the port cannot be bound or
    /// the CA file cannot be written.
    pub async fn bind() -> Result<Self, ListenerError> {
        let acceptor = TlsAcceptor::from(server_config()?);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        // NOTE: the bound port and the process id make the file one listener's own.
        let ca_file = std::env::temp_dir().join(format!(
            "ferroehr-test-ca-{}-{port}.pem",
            std::process::id()
        ));
        tokio::fs::write(&ca_file, TEST_CA_PEM).await?;
        let (tx, frames) = mpsc::unbounded_channel();
        drop(tokio::spawn(async move {
            while let Ok((tcp, _peer)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let tx = tx.clone();
                drop(tokio::spawn(async move {
                    if let Ok(tls) = acceptor.accept(tcp).await {
                        read_frames(tls, &tx).await;
                    }
                }));
            }
        }));
        Ok(Self {
            port,
            ca_file,
            frames,
        })
    }

    /// The next whole frame, `MSG-LEN SP SYSLOG-MSG`, or `None` when none
    /// arrives within `wait`.
    pub async fn recv(&mut self, wait: Duration) -> Option<Vec<u8>> {
        tokio::time::timeout(wait, self.frames.recv())
            .await
            .ok()
            .flatten()
    }
}

/// Reads RFC 5425 §4.3 frames off `stream` until it ends, sending each.
async fn read_frames<S: tokio::io::AsyncRead + Unpin>(
    mut stream: S,
    tx: &mpsc::UnboundedSender<Vec<u8>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        while let Some(frame) = take_frame(&mut buffer) {
            if tx.send(frame).is_err() {
                return;
            }
        }
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(read) => buffer.extend(chunk.iter().take(read)),
        }
    }
}

/// The first whole frame in `buffer`, removed from it.
fn take_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let space = buffer.iter().position(|byte| *byte == b' ')?;
    let length: usize = std::str::from_utf8(buffer.get(..space)?)
        .ok()?
        .parse()
        .ok()?;
    let end = space.checked_add(1)?.checked_add(length)?;
    if buffer.len() < end {
        return None;
    }
    Some(buffer.drain(..end).collect())
}

/// The server configuration presenting the test leaf.
fn server_config() -> Result<Arc<rustls::ServerConfig>, ListenerError> {
    use rustls::pki_types::pem::PemObject as _;
    let pki = |e: rustls::Error| ListenerError::Pki(Box::new(e));
    let certs: Vec<_> = rustls::pki_types::CertificateDer::pem_slice_iter(TEST_LEAF_PEM)
        .collect::<Result<_, _>>()
        .map_err(|e| ListenerError::Pki(Box::new(e)))?;
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(TEST_LEAF_KEY_PEM)
        .map_err(|e| ListenerError::Pki(Box::new(e)))?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(pki)?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(pki)?;
    Ok(Arc::new(config))
}
