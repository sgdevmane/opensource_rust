// =============================================================================
// DataForge REST Server — Axum API & OpenAPI Swagger Documentation
// =============================================================================

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    response::{IntoResponse, Html},
    routing::{get, post},
    Json, Router,
};
use dataforge_core::metrics::CoreMetricsRegistry;
use dataforge_core::transform::clean::{DataCleaner, CleanStrategy};
use dataforge_core::transform::diff::WorkbookDiffEngine;
use dataforge_core::convert::PdfReportGenerator;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

#[derive(Clone)]
pub struct AppState {
    pub metrics: Arc<CoreMetricsRegistry>,
}

#[derive(OpenApi)]
#[openapi(
    paths(
        health_check,
        get_metrics,
        evaluate_formula,
        clean_dataset,
        generate_pdf_report
    ),
    components(schemas(
        HealthResponse,
        FormulaRequest,
        FormulaResponse,
        CleanRequest,
        CleanResponse,
        PdfReportRequest
    )),
    tags((name = "DataForge Engine", description = "High-performance spreadsheet parsing & transformation REST API"))
)]
struct ApiDoc;

#[derive(Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct FormulaRequest {
    pub formula: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct FormulaResponse {
    pub result: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct CleanRequest {
    pub sample_text: String,
    pub strategy: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct CleanResponse {
    pub cleaned_text: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub struct PdfReportRequest {
    pub title: String,
    pub dark_mode: bool,
}

#[utoipa::path(
    get,
    path = "/health",
    responses((status = 200, description = "Service Health Check", body = HealthResponse))
)]
async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "OK".into(),
        service: "dataforge-server".into(),
        version: "0.1.0".into(),
    })
}

#[utoipa::path(
    get,
    path = "/metrics",
    responses((status = 200, description = "Prometheus Metrics Text Exposition"))
)]
async fn get_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let metrics_text = state.metrics.render_prometheus();
    (StatusCode::OK, [("content-type", "text/plain; version=0.0.4")], metrics_text)
}

#[utoipa::path(
    post,
    path = "/api/v1/evaluate",
    request_body = FormulaRequest,
    responses((status = 200, description = "Evaluated formula result", body = FormulaResponse))
)]
async fn evaluate_formula(Json(payload): Json<FormulaRequest>) -> Json<FormulaResponse> {
    let evaluator = dataforge_core::FormulaEvaluator::new();
    let res = evaluator.eval_str(&payload.formula).unwrap_or_else(|e| format!("Error: {e}"));
    Json(FormulaResponse { result: res })
}

#[utoipa::path(
    post,
    path = "/api/v1/clean",
    request_body = CleanRequest,
    responses((status = 200, description = "Cleaned/Anonymized string result", body = CleanResponse))
)]
async fn clean_dataset(Json(payload): Json<CleanRequest>) -> Json<CleanResponse> {
    let mut batch = dataforge_core::types::RowBatch {
        schema: dataforge_core::types::Schema { fields: vec![] },
        rows: vec![dataforge_core::types::Row {
            cells: vec![dataforge_core::types::CellValue::String(payload.sample_text.into())],
        }],
    };

    let strategy = match payload.strategy.as_str() {
        "mask_email" => CleanStrategy::MaskEmail,
        "mask_phone" => CleanStrategy::MaskPhone,
        "mask_ssn" => CleanStrategy::MaskSsn,
        "mask_cc" => CleanStrategy::MaskCreditCard,
        "uppercase" => CleanStrategy::Uppercase,
        "lowercase" => CleanStrategy::Lowercase,
        _ => CleanStrategy::TrimWhitespace,
    };

    let cleaner = DataCleaner::new().add_rule(0, strategy);
    cleaner.clean_batch(&mut batch);

    let cleaned = batch.rows[0].cells[0].to_display_string();
    Json(CleanResponse { cleaned_text: cleaned })
}

#[utoipa::path(
    post,
    path = "/api/v1/report/pdf",
    request_body = PdfReportRequest,
    responses((status = 200, description = "Rendered HTML/PDF Report"))
)]
async fn generate_pdf_report(Json(payload): Json<PdfReportRequest>) -> Html<String> {
    let generator = PdfReportGenerator::new(payload.title).with_dark_mode(payload.dark_mode);
    let sample_batch = dataforge_core::types::RowBatch {
        schema: dataforge_core::types::Schema {
            fields: vec![
                dataforge_core::types::Field { name: "System".into(), data_type: dataforge_core::types::DataType::String },
                dataforge_core::types::Field { name: "Status".into(), data_type: dataforge_core::types::DataType::String },
            ],
        },
        rows: vec![dataforge_core::types::Row {
            cells: vec![
                dataforge_core::types::CellValue::String("DataForge Core".into()),
                dataforge_core::types::CellValue::String("Operational".into()),
            ],
        }],
    };
    let html = generator.render_html(&sample_batch).unwrap_or_default();
    Html(html)
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    let state = AppState {
        metrics: Arc::new(CoreMetricsRegistry::new()),
    };

    let app = Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/health", get(health_check))
        .route("/metrics", get(get_metrics))
        .route("/api/v1/evaluate", post(evaluate_formula))
        .route("/api/v1/clean", post(clean_dataset))
        .route("/api/v1/report/pdf", post(generate_pdf_report))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .unwrap_or_else(|_| "8080".to_string())
        .parse()
        .unwrap_or(8080);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("🚀 DataForge REST API server running on http://{}", addr);
    println!("📖 Swagger UI documentation available at http://{}/swagger-ui", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
