// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer server binary: config → CDR client → (optional) OIDC
//! discovery → the assembled service ([`ferroehr_viewer::server::router`])
//! → serve. Wiring only — logic lives in the lib.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use leptos::prelude::LeptosOptions;

    // Console logs: env-filtered (RUST_LOG), stderr. Without a subscriber
    // every server-side error is invisible — an operational defect.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // `healthcheck` subcommand: the distroless image has no shell, so the
    // container healthcheck is the binary probing its own /login over HTTP
    // (exit 0 on 2xx/3xx). The bind address comes from LEPTOS_SITE_ADDR,
    // matching the serving path below.
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        #[expect(
            clippy::disallowed_methods,
            reason = "LEPTOS_SITE_ADDR is cargo-leptos's own variable, read here for the same reason leptos::config::get_configuration reads it below — the probe must hit the address this process serves on"
        )]
        let addr =
            std::env::var("LEPTOS_SITE_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_owned());
        let addr = addr.replace("0.0.0.0", "127.0.0.1");
        let status = reqwest::Client::new()
            .get(format!("http://{addr}/login"))
            .send()
            .await?
            .status();
        anyhow::ensure!(
            status.is_success() || status.is_redirection(),
            "healthcheck: /login answered {status}"
        );
        return Ok(());
    }

    let config =
        std::sync::Arc::new(ferroehr_viewer::config::load().map_err(|e| anyhow::anyhow!("{e}"))?);
    let cdr =
        ferroehr_viewer::cdr::CdrClient::new(&config.cdr).map_err(|e| anyhow::anyhow!("{e}"))?;
    let oidc = if config.auth.oidc.enabled {
        Some(std::sync::Arc::new(
            ferroehr_viewer::oidc::discover(&config.auth.oidc).await?,
        ))
    } else {
        None
    };
    let session_keys = ferroehr_viewer::session::SessionKeys::from_secret(&config.session.secret)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let app_state = ferroehr_viewer::state::AppState {
        config: std::sync::Arc::clone(&config),
        cdr,
        oidc,
        session_keys,
    };

    let conf = leptos::config::get_configuration(None)?;
    let leptos_options: LeptosOptions = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let service = ferroehr_viewer::server::router(app_state, leptos_options);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "ferroehr-viewer listening");
    axum::serve(listener, service.into_make_service()).await?;
    Ok(())
}

#[cfg(not(feature = "ssr"))]
fn main() {
    // The binary is only meaningful with `ssr`; the WASM client uses
    // `lib.rs::hydrate`. cargo-leptos always builds the bin with `ssr`.
}
