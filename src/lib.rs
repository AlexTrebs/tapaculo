use axum::{http::{header::CONTENT_TYPE, Method}, Router};
use dotenv::dotenv;
use tokio::net::TcpListener;
use tower_http::{compression::CompressionLayer, cors::{Any, CorsLayer}, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub async fn app() -> Result<Router, ()> {
  let cors = CorsLayer::new()
    .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::PUT])
    .allow_origin(Any)
    .allow_headers([CONTENT_TYPE]);
  
  let app: Router = Router::new()
    .layer(cors)
    .layer(TraceLayer::new_for_http())
    .layer(CompressionLayer::new());

  Ok(app)
}

pub async fn run() {
  dotenv().ok();

  tracing_subscriber::registry()
    .with(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| format!(
            "{}=debug,tower_http=debug,axum::rejection=trace",
            env!("CARGO_CRATE_NAME")
          ).into()),
    )
    .with(tracing_subscriber::fmt::layer())
    .init();

  let port = std::env::var("PORT").unwrap_or(String::from("3000"));

  let addr = format!("0.0.0.0:{}", port);
  info!("listening on {}", addr);

  let listener = TcpListener::bind(addr).await.unwrap();

  let app = app().await.unwrap();

  axum::serve(listener, app).await.unwrap();
}