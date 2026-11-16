use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use rayon::prelude::*;
use scprust_core::{
    Agent, CoreError, Observer, Publish, RouteQuery, Version, World, WorldStore, MAX_BATCH,
};
use scprust_workers::{compute, validate, Batch, QueryResult};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{io::Read, net::SocketAddr, sync::Arc};
use tokio::sync::Semaphore;

struct App {
    token: String,
    pool: Arc<rayon::ThreadPool>,
    slots: Arc<Semaphore>,
    world: WorldStore,
}

type ApiError = (StatusCode, &'static str);

fn core_error(error: CoreError) -> ApiError {
    match error {
        CoreError::Invalid(message) => (StatusCode::BAD_REQUEST, message),
        CoreError::Conflict => (
            StatusCode::CONFLICT,
            "world version conflict; reconnect and republish",
        ),
        CoreError::Unavailable => (StatusCode::INTERNAL_SERVER_ERROR, "world unavailable"),
    }
}

fn authorize(app: &App, headers: &HeaderMap) -> Result<(), ApiError> {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some(app.token.as_str()) {
        return Err((StatusCode::UNAUTHORIZED, "unauthorized"));
    }
    Ok(())
}

/// All API CPU jobs share one admission budget and one Rayon pool. A client
/// disconnect cannot release its permit while computation is still running.
async fn job<T, R, F>(
    app: Arc<App>,
    headers: HeaderMap,
    body: Bytes,
    work: F,
) -> Result<Json<R>, ApiError>
where
    T: DeserializeOwned + Send + 'static,
    R: Serialize + Send + 'static,
    F: FnOnce(T, &App) -> Result<R, ApiError> + Send + 'static,
{
    authorize(&app, &headers)?;
    let permit = app
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| (StatusCode::TOO_MANY_REQUESTS, "workers busy"))?;
    let pool = app.pool.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let input: T = serde_json::from_slice(&body)
            .map_err(|_| (StatusCode::BAD_REQUEST, "invalid JSON or schema"))?;
        pool.install(|| work(input, &app))
    })
    .await
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "worker failed"))??;
    Ok(Json(result))
}

#[derive(Serialize)]
struct Results<T> {
    results: Vec<T>,
}

async fn radius(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Results<QueryResult>>, ApiError> {
    job(app, headers, body, |batch: Batch, _| {
        validate(&batch).map_err(|message| (StatusCode::BAD_REQUEST, message))?;
        Ok(Results {
            results: compute(batch),
        })
    })
    .await
}

#[derive(Serialize)]
struct WorldInfo {
    version: Version,
    entities: usize,
    nodes: usize,
}
impl WorldInfo {
    fn from_world(world: &World) -> Self {
        Self {
            version: world.version.clone(),
            entities: world.entity_count(),
            nodes: world.navigation.node_count(),
        }
    }
}

async fn world_info(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Result<Json<WorldInfo>, ApiError> {
    authorize(&app, &headers)?;
    let world = app.world.latest().map_err(core_error)?;
    Ok(Json(WorldInfo::from_world(&world)))
}

async fn publish(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WorldInfo>, ApiError> {
    job(app, headers, body, |update: Publish, app| {
        let world = app.world.publish(update).map_err(core_error)?;
        Ok(WorldInfo::from_world(&world))
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CoreBatch<T> {
    version: Version,
    items: Vec<T>,
}
impl<T> CoreBatch<T> {
    fn world(&self, store: &WorldStore) -> Result<Arc<World>, ApiError> {
        if self.items.is_empty() || self.items.len() > MAX_BATCH {
            return Err((StatusCode::BAD_REQUEST, "core batches require 1..128 items"));
        }
        store.snapshot(&self.version).map_err(core_error)
    }
}

#[derive(Serialize)]
struct CoreResults<T> {
    version: Version,
    results: Vec<T>,
}

async fn think(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<CoreResults<scprust_core::Intent>>, ApiError> {
    job(app, headers, body, |batch: CoreBatch<Agent>, app| {
        let world = batch.world(&app.world)?;
        for agent in &batch.items {
            world.validate_agent(agent).map_err(core_error)?;
        }
        let results: Result<Vec<_>, _> = batch
            .items
            .par_iter()
            .map(|agent| world.think(agent))
            .collect();
        Ok(CoreResults {
            version: world.version.clone(),
            results: results.map_err(core_error)?,
        })
    })
    .await
}

async fn routes(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<CoreResults<scprust_core::Route>>, ApiError> {
    job(app, headers, body, |batch: CoreBatch<RouteQuery>, app| {
        let world = batch.world(&app.world)?;
        for query in &batch.items {
            world.navigation.validate_query(query).map_err(core_error)?;
        }
        let results: Result<Vec<_>, _> = batch
            .items
            .par_iter()
            .map(|query| world.navigation.route(query))
            .collect();
        Ok(CoreResults {
            version: world.version.clone(),
            results: results.map_err(core_error)?,
        })
    })
    .await
}

async fn interest(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<CoreResults<scprust_core::Interest>>, ApiError> {
    job(app, headers, body, |batch: CoreBatch<Observer>, app| {
        let world = batch.world(&app.world)?;
        for observer in &batch.items {
            world.validate_observer(observer).map_err(core_error)?;
        }
        let results: Result<Vec<_>, _> = batch
            .items
            .par_iter()
            .map(|observer| world.interest(observer))
            .collect();
        Ok(CoreResults {
            version: world.version.clone(),
            results: results.map_err(core_error)?,
        })
    })
    .await
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var("SCPRUST_TOKEN").map_err(|_| "SCPRUST_TOKEN must be set")?;
    if token.len() < 32 || !token.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("SCPRUST_TOKEN must contain at least 32 ASCII letters/digits".into());
    }
    let threads: usize = std::env::var("SCPRUST_THREADS")
        .unwrap_or_else(|_| "4".into())
        .parse()?;
    if !(1..=64).contains(&threads) {
        return Err("SCPRUST_THREADS must be between 1 and 64".into());
    }
    let address: SocketAddr = std::env::var("SCPRUST_BIND")
        .unwrap_or_else(|_| "127.0.0.1:27185".into())
        .parse()?;
    if !address.ip().is_loopback() {
        return Err("only loopback binds are supported".into());
    }
    let mut epoch_bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut epoch_bytes)?;
    let epoch: String = epoch_bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let app = Arc::new(App {
        token: format!("Bearer {token}"),
        pool: Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()?,
        ),
        slots: Arc::new(Semaphore::new(2)),
        world: WorldStore::new(epoch),
    });
    let router = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/v1/radius", post(radius))
        .route("/v1/world", get(world_info).post(publish))
        .route("/v1/think", post(think))
        .route("/v1/routes", post(routes))
        .route("/v1/interest", post(interest))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(address).await?;
    eprintln!("scprust-workers listening on {address}, {threads} compute workers");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
