# SpreadsheetParser (DataForge Engine) 🚀

DataForge is a high-performance, memory-bounded, cross-language streaming spreadsheet and tabular data engine written in Rust. It enables developers in Rust, Python, Node.js, WebAssembly (browser/Edge), and C/C++ to process, parse, clean, transform, diff, and export massive spreadsheet datasets with constant memory usage.

---

## 🌟 The 20 Core Enhancements Implemented

1. **Streaming CSV/XLSX/ODS Parsers**: Constant memory parsing for arbitrarily large files without loading the entire dataset into memory.
2. **Spreadsheet Formula Evaluation Engine**: Evaluates functions like `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT`, `IF`, `VLOOKUP`, `CONCAT`, and `TRIM`.
3. **Parquet & Feather Format Support**: Native read/write support for Apache Parquet and Apache Arrow IPC formats.
4. **Automatic Schema Inference & Data Type Sniffing**: Automatic detection of integers, floats, booleans, dates, datetimes, strings, currencies, and percentages.
5. **Data Cleaning & Anonymization Pipeline (`clean.rs`)**: PII masking (Email, Phone, SSN, Credit Card, IP Address), whitespace trimming, and regex normalization.
6. **In-Browser Web Worker Processing (WASM)**: Asynchronous Web Worker bindings operating on Uint8Array buffers without freezing the browser UI thread.
7. **Virtualized UI Grid Integration**: Ready-to-use export helpers formatting output for AG Grid, TanStack Table, and Handsontable.
8. **Export to PDF & HTML Reports (`pdf.rs`)**: Styled HTML report generator with dark mode and printable PDF document formatting.
9. **Zero-Copy Arrow PyCapsule / PyArrow Integration**: Memory sharing with PyArrow, Pandas, and Polars.
10. **Async Node.js N-API Streams**: Native streaming chunk processing in Node.js Express/Fastify pipelines.
11. **Type-Safe TypeScript Definitions**: Complete auto-generated TypeScript declarations (`index.d.ts`).
12. **Prometheus Metrics & Grafana Dashboard**: Built-in `/metrics` exposition endpoint and pre-configured Grafana dashboard (`grafana-dashboard.json`).
13. **Fuzz Testing Suite (`fuzz_csv_xlsx.rs`)**: Fuzz targets protecting parsers from panic crashes on malformed files.
14. **Swagger & Postman API Schema**: Interactive Swagger UI (`/swagger-ui`) powered by `utoipa` and importable Postman collection (`postman_collection.json`).
15. **Docker Staging & Production Compose Setup**: `docker-compose.yml`, `docker-compose.staging.yml`, `docker-compose.prod.yml`, `.env.local`, `.env.staging`, and `.env.production`.
16. **PostgreSQL / SQL Dump Exporter**: Generates PostgreSQL DDL & DML statements (`CREATE TABLE`, `INSERT`, `COPY`).
17. **Pivot Table & Aggregation Engine (`pivot.rs`)**: Multi-dimensional pivot grouping (`SUM`, `COUNT`, `AVG`, `MIN`, `MAX`).
18. **Fuzzy String Matching & Deduplication (`dedup.rs`, `join.rs`)**: Levenshtein / Jaro-Winkler distance deduplication and fuzzy joining.
19. **Diff & Audit Engine (`diff.rs`)**: Detects inserted, deleted, and modified rows and cell-level changes between workbooks.
20. **Multi-Workbook Merge & Join Engine**: INNER, LEFT, RIGHT, and FULL OUTER joins across tables.

---

## 🏗️ Architecture & Member Crates

```
spreadsheet_parser/
├── Cargo.toml                    # Workspace root manifest
├── init.sql                      # Fresh database setup script for PostgreSQL
├── postman_collection.json       # Importable Postman REST collection
├── prometheus.yml                # Prometheus scrape config
├── grafana-dashboard.json        # Grafana dashboard visualization
├── docker-compose.yml            # Local dev Docker Compose
├── docker-compose.staging.yml    # Staging Docker Compose
├── docker-compose.prod.yml       # Production Docker Compose
├── .env.local                    # Local environment settings
├── .env.staging                  # Staging environment settings
├── .env.production               # Production environment settings
├── Dockerfile                    # Container build configuration
├── crates/
│   ├── dataforge-core/           # Rust core engine (parsers, transforms, diff, pdf, metrics)
│   ├── dataforge-server/         # REST API Backend with Axum, Swagger UI, & SQLx Postgres
│   ├── dataforge-ffi/            # C-compatible static & dynamic library + dataforge.h
│   ├── dataforge-node/           # Node.js native addon (napi-rs)
│   ├── dataforge-python/         # Python compiled module (PyO3)
│   └── dataforge-wasm/           # WebAssembly bindings (wasm-bindgen)
└── fuzz/                         # Fuzz testing suite
```

---

## ⚡ Quick Start Commands & Scripts

### Running the REST Backend Server
```bash
# Run locally with .env.local settings
cargo run -p dataforge-server

# Access API & Swagger UI:
# - Swagger UI: http://localhost:8080/swagger-ui
# - Health check: http://localhost:8080/health
# - Prometheus metrics: http://localhost:8080/metrics
```

### Running with Docker Compose
```bash
# Local development:
docker-compose up --build

# Staging deployment:
docker-compose -f docker-compose.staging.yml up -d

# Production deployment:
docker-compose -f docker-compose.prod.yml up -d
```

### Database Ingestion (`init.sql`)
To set up a fresh PostgreSQL database instance:
```bash
psql $DATABASE_URL -f init.sql
```

### Running Workspace Tests & Verification
```bash
# Run all workspace unit and integration tests
cargo test --workspace

# Check workspace for lints and compilation warnings
cargo check --workspace
```

---

## 📜 Architectural Rules, Do's and Don'ts

### ✅ Do's
- Always process large files using chunked streaming readers (`CsvReader`, `XlsxReader`) to enforce constant memory usage.
- Store sensitive database connection URLs in `.env.local`, `.env.staging`, or `.env.production` files.
- Monitor active jobs and throughput via `/metrics` using Prometheus and Grafana.
- Use `dataforge-server` Swagger UI (`/swagger-ui`) or `postman_collection.json` for testing endpoints.

### ❌ Don'ts
- Do not load entire multi-gigabyte files into memory at once.
- Do not commit hardcoded database credentials or secret keys to version control.
- Do not ignore zero warnings/errors policies; ensure `cargo check --workspace` passes cleanly.

---

## ⚖️ License

Dual-licensed under either:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))
