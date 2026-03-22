use std::net::SocketAddrV4;

use anyhow::Result;
use tokio::{net::TcpListener, spawn};
use tracing::{info, warn};
use utoipa_rapidoc::RapiDoc;

use crate::{api, config::Config, frontend, scheduler};

pub async fn run(config: Config, host: String, port: u32) -> Result<()> {
    // Start scheduler
    let tasks_path = config.tasks_path.clone();
    spawn(async move {
        if let Err(e) = scheduler::run(&tasks_path).await {
            warn!(?e, "scheduler exited with error");
        }
    });

    // Set up API server
    let (router, api) = api::router(&config).split_for_parts();
    let router = router
        .merge(RapiDoc::with_openapi("/api-docs/openapi.json", api).path("/rapidoc"))
        .fallback_service(frontend::router());

    let address: SocketAddrV4 = format!("{host}:{port}").parse()?;
    let listener = TcpListener::bind(address).await?;

    // Start the web server
    info!(%host, %port, "starting web server");
    axum::serve(listener, router).await?;
    Ok(())
}
