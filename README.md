# DataForge 🚀

DataForge is a high-performance, memory-bounded, cross-language streaming spreadsheet engine written in Rust. It allows developers in any language—including Node.js, Python, C/C++, and WebAssembly (browsers/Edge)—to parse, write, and manipulate massive spreadsheet datasets (100k+ to millions of rows in CSV, XLSX, and ODS formats) with constant memory usage, avoiding Out-Of-Memory (OOM) crashes.

---

## Why DataForge Wins

- **Memory-Bounded Operations**: DataForge operates on a configurable memory ceiling (e.g. 256MB) and employs a thread-safe atomic backpressure mechanism. If memory utilization is high, producer threads are blocked or throttled, guaranteeing constant memory consumption.
- **Zero-Copy & SIMD Parsing**: Utilizes memory-mapped files via `memmap2` and SIMD-accelerated scanning (`memchr`) to process delimiters and boundaries at raw hardware speed.
- **Multi-Core Scaling**: Parallelizes CSV chunk boundaries dynamically across available CPU cores via Rayon's work-stealing scheduler.
- **True SAX XML Parsing**: XLSX and ODS streaming parsers read ZIP xml elements row-by-row, resolving shared strings lazily without ever holding a full XML DOM in memory.
- **Composable Transformation Pipeline**: Supports lazy, streaming transformations (filter, map, type-coercion, computed columns, sorted batches, and aggregates) on raw row batches.
- **Universal Portability**: Native Rust core wrapped with target-specific bindings:
  - **Rust**: Safe zero-cost abstractions
  - **C FFI**: Stable ABI for C, C++, Go, C#, Java (JNI), Ruby, C#
  - **Node.js**: `napi-rs` bindings with async generators and JS proxy objects
  - **Python**: `PyO3` bindings with iterator/generator protocols releasing the GIL
  - **WebAssembly**: `wasm-bindgen` bindings for browser/Edge workers operating on ArrayBuffers

---

## Workspace Structure

The workspace is organized as a Cargo workspace with 5 specialized crates:

```
opensource_rust/
├── Cargo.toml                    # Workspace root Cargo manifest
├── LICENSE-MIT                   # MIT License
├── LICENSE-APACHE                # Apache 2.0 License
├── README.md                     # This file
├── CONTRIBUTING.md               # Contribution Guidelines
├── crates/
│   ├── dataforge-core/           # Pure Rust core implementation
│   ├── dataforge-ffi/            # C-compatible ABI static & dynamic library
│   ├── dataforge-node/           # Node.js native addon (napi-rs)
│   ├── dataforge-python/         # Python compiled module (PyO3)
│   └── dataforge-wasm/           # Browser & Edge WASM package (wasm-bindgen)
├── examples/                     # Runable language-specific examples
└── tests/                        # Workspace integration tests
```

---

## Performance Targets & Benchmarks

| Operation | Performance Target (1M Rows) | Memory Footprint |
| :--- | :--- | :--- |
| **CSV Read** | < 2.0 seconds | Constant (~30MB) |
| **CSV Parallel Read** | < 0.5 seconds | Constant (~50MB) |
| **XLSX Read** | < 8.0 seconds | Constant (~60MB) |
| **CSV → XLSX Convert** | < 15.0 seconds | Constant (~80MB) |

*Benchmarks ran on Apple M-series chips using `criterion` framework.*

---

## Quick Start & Examples

Detailed execution examples for all supported environments are provided below.

### 1. Rust Core Usage

Add the library to your `Cargo.toml`:
```toml
[dependencies]
dataforge-core = { path = "./crates/dataforge-core" }
```

Stream a CSV file:
```rust
use dataforge_core::config::ReaderConfig;
use dataforge_core::csv::CsvReader;

fn main() {
    let config = ReaderConfig::default()
        .with_batch_size(8192)
        .with_parallel(true);

    let reader = CsvReader::open("massive_data.csv", config).unwrap();
    for batch_result in reader {
        let batch = batch_result.unwrap();
        println!("Loaded batch of {} rows", batch.len());
        for row in batch.iter() {
            // Access cells by index:
            let name = row.get_str(0);
            let age = row.get_int(1);
        }
    }
}
```

### 2. Node.js Bindings (napi-rs)

Install directly from the npm registry (precompiled binaries are loaded dynamically):
```bash
npm install dataforge-native
```

Alternatively, to compile the Node bindings from source:
```bash
cd crates/dataforge-node
npm install
npm run build
```

Usage in JavaScript:
```javascript
import { JsCsvReader } from 'dataforge-native';

const reader = JsCsvReader.open('massive_data.csv', 8192, true);
console.log('Headers:', reader.headers);

let batch;
while ((batch = reader.next_batch()) !== null) {
  const jsonRows = batch.toJsonObjects(); // Converts rows to plain JS objects
  console.log(`Processed ${jsonRows.length} rows`);
}
```

### 3. Python Bindings (PyO3)

Compile Python bindings using Maturin:
```bash
cd crates/dataforge-python
pip install maturin
maturin develop
```

Usage in Python:
```python
import dataforge

# PyCsvReader yields batches incrementally, releasing the GIL during parsing
reader = dataforge.PyCsvReader("massive_data.csv", batch_size=8192)
for batch in reader:
    # to_dicts() returns list of dicts, ready to be read into Pandas/Polars
    records = batch.to_dicts()
    print(f"Processed batch of {len(records)} records")
```

### 4. WebAssembly Bindings (wasm-bindgen)

Compile for target browsers or bundlers:
```bash
cd crates/dataforge-wasm
wasm-pack build --target web
```

Usage in the browser/worker:
```javascript
import init, { WasmXlsxReader } from './pkg/dataforge_wasm.js';

await init();

// Read spreadsheet bytes uploaded by user
const response = await fetch('upload.xlsx');
const bytes = new Uint8Array(await response.arrayBuffer());

const reader = new WasmXlsxReader(bytes, 4096);
let batch;
while ((batch = reader.next_batch()) !== null) {
  const data = batch.to_json_objects();
  console.log(data);
}
```

### 5. C / C++ & FFI Bindings

Build static and dynamic C libraries:
```bash
cd crates/dataforge-ffi
cargo build --release
```
This produces `libdataforge_ffi.a` (static) and `libdataforge_ffi.so` / `libdataforge_ffi.dylib` (dynamic) and automatically generates the header file `crates/dataforge-ffi/include/dataforge.h` using `cbindgen` to link in C/C++, Go, or Python `ctypes`.

---

## Advanced Features & Core Upgrades

DataForge is packed with production-ready, premium-grade features:

### 1. Password Protected XLSX Decryption & Encryption
Transparently handles ECMA-376 Agile password-protected spreadsheets:
- **Decryption**: Provide the decryption password in the configuration:
  ```rust
  let config = ReaderConfig::default().with_password("my_secure_password");
  let reader = XlsxReader::open("encrypted.xlsx", config)?;
  ```
- **Encryption**: Export unencrypted XLSX payloads to password-protected OLE documents:
  ```rust
  dataforge_core::xlsx::encrypt_xlsx(&xlsx_bytes, "password", &mut file)?;
  ```

### 2. Composable Transformation Pipelines
Lazily build high-performance data processing pipelines:
```rust
use dataforge_core::transform::pipeline::Pipeline;
use dataforge_core::transform::filter::{ColumnFilter, ComparisonOp};

let pipeline = Pipeline::new()
    .filter("Age", ComparisonOp::GreaterThan, CellValue::from(30_i64))
    .rename_column("Name", "Full Name")
    .select_columns(vec!["Full Name".to_string(), "Age".to_string()]);
```

### 3. Disk-Buffered Sorting & Incremental Runs
Sort datasets larger than memory using external merge sort:
```rust
// Sorts by column "Age" descending using temporary disk runs
let mut sorted_reader = reader.external_sort("Age", false)?;
```

### 4. Excel Styling Templates
Generate beautifully styled workbooks instantly with professional styling presets:
```rust
use dataforge_core::xlsx::StyleTemplate;

let config = WriterConfig::default()
    .with_style_template(StyleTemplate::Professional); // Navy headers, zebra shading, frozen headers, auto-filters
```

### 5. Observability (Prometheus & Grafana)
Memory consumption is fully instrumented for system telemetry:
```rust
// Export memory statistics in Prometheus text exposition format
let metrics_payload = memory_tracker.to_prometheus();
```

---

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) for details on code style, linting, formatting, testing conventions, and pull request processes.

## License

Dual-licensed under either:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
