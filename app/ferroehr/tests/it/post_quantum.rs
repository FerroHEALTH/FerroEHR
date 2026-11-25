// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The key exchange of the outbound HTTPS clients FerroEHR builds: each
//! prefers the hybrid X25519MLKEM768 (RFC 10024) and agrees on X25519 with a
//! peer that lacks it. The outbound integration client is the one the ITI-20
//! feed, the PIXm query, the PMIR subscription and the viewer build with
//! `OutboundClient::build`; the S3 client is the multimedia store's own. The
//! peer's certificate is trusted by no root, so each call ends after the key
//! exchange.

use ferroehr_ext::multimedia::blob::{
    BackendSettings, BlobKey, BlobStore, BlobStoreParams, Limits, S3Settings,
};
use ferrohealth_smart::client::outbound::OutboundClient;
use ferrohealth_smart::config::ServerTls;
use ferrohealth_smart::config::endpoint::EndpointSection;
use rustls::NamedGroup;
use rustls::crypto::SupportedKxGroup;
use rustls::crypto::aws_lc_rs::kx_group;
use testkit::kx::KxPeer;

/// The peer groups of the two cases: one that offers the hybrid after the
/// classical group, so the client's preference decides, and one that offers
/// no hybrid.
fn peers() -> [(Vec<&'static dyn SupportedKxGroup>, NamedGroup); 2] {
    [
        (
            vec![kx_group::X25519, kx_group::X25519MLKEM768],
            NamedGroup::X25519MLKEM768,
        ),
        (vec![kx_group::X25519], NamedGroup::X25519),
    ]
}

/// The group the outbound integration client agreed on with `peer`.
async fn agreed_by_the_outbound_client(peer: &KxPeer) -> Option<NamedGroup> {
    let url = format!("https://{}/", peer.address());
    let endpoint = EndpointSection {
        url: ferrohealth_secret::secret_url::SecretUrl::new(&url),
        ..EndpointSection::default()
    };
    let client = OutboundClient::build(&endpoint, "terminology.endpoint", &ServerTls)
        .expect("the client builds");
    let answer = client.http().get(&url).send().await;
    assert!(
        answer.is_err(),
        "the peer's certificate is trusted by no root"
    );
    peer.agreed(0).expect("the connection is recorded")
}

/// The group the multimedia store's S3 client agreed on with `peer`.
async fn agreed_by_the_s3_client(peer: &KxPeer) -> Option<NamedGroup> {
    let store = BlobStore::from_params(BlobStoreParams {
        backend: BackendSettings::S3(S3Settings {
            endpoint: Some(
                url::Url::parse(&format!("https://{}/", peer.address())).expect("a URL"),
            ),
            bucket: "openehr-multimedia".to_owned(),
            region: "us-east-1".to_owned(),
            access_key_id: None,
            secret_access_key: None,
            allow_http: false,
        }),
        limits: Limits::default(),
    })
    .expect("the store builds");
    let answer = store.exists(&BlobKey::of(b"post-quantum")).await;
    assert!(
        answer.is_err(),
        "the peer's certificate is trusted by no root"
    );
    peer.agreed(0).expect("the connection is recorded")
}

#[tokio::test]
async fn the_outbound_client_prefers_x25519mlkem768_and_falls_back_to_x25519() {
    for (groups, want) in peers() {
        let peer = KxPeer::start(groups).expect("the peer starts");
        assert_eq!(Some(want), agreed_by_the_outbound_client(&peer).await);
    }
}

#[tokio::test]
async fn the_s3_client_prefers_x25519mlkem768_and_falls_back_to_x25519() {
    for (groups, want) in peers() {
        let peer = KxPeer::start(groups).expect("the peer starts");
        assert_eq!(Some(want), agreed_by_the_s3_client(&peer).await);
    }
}
