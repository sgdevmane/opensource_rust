# Contributing to SpreadsheetParser

Thank you for considering contributing to SpreadsheetParser! This document provides guidelines
and instructions for contributing to the project.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Project Architecture](#project-architecture)
- [Making Changes](#making-changes)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Submitting Changes](#submitting-changes)
- [Release Process](#release-process)

## Code of Conduct

This project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct).
Be respectful, constructive, and collaborative.

## Getting Started

### Prerequisites

- **Rust** (stable, latest): Install via [rustup](https://rustup.rs/)
- **Node.js** (v18+): Required for `dataforge-node` bindings
- **Python** (3.9+): Required for `dataforge-python` bindings
- **wasm-pack**: Required for `dataforge-wasm` — install via `cargo install wasm-pack`
- **maturin**: Required for `dataforge-python` — install via `pip install maturin`

### First-Time Setup

```bash
# Clone the repository
git clone https://github.com/sgdevmane/spreadsheet_parser.git
cd spreadsheet_parser

# Build the entire workspace
cargo build --workspace

# Run all tests
cargo test --workspace

# Run clippy lints
cargo clippy --workspace --all-targets -- -D warnings

# Format check
cargo fmt --check --all
```

## Development Setup

### Building Individual Crates

```bash
# Core library only
cargo build -p dataforge-core

# C FFI library (generates dataforge.h header)
cargo build -p dataforge-ffi

# Node.js bindings
cd crates/dataforge-node
npm install
npm run build

# Python bindings
cd crates/dataforge-python
maturin develop

# WASM bindings
cd crates/dataforge-wasm
wasm-pack build --target web
```

### Running Benchmarks

```bash
# Run all benchmarks
cargo bench --workspace

# Run specific benchmark
cargo bench --bench csv_read

# Generate benchmark report (opens in browser)
cargo bench --workspace -- --output-format html
```

## Project Architecture

```
spreadsheet_parser/
├── crates/
│   ├── dataforge-core/      # Pure Rust core — ALL logic lives here
│   │   ├── src/
│   │   │   ├── csv/         # CSV read/write (streaming, parallel)
│   │   │   ├── xlsx/        # XLSX read/write (SAX-style XML streaming)
│   │   │   ├── ods/         # ODS read/write (OpenDocument Spreadsheet)
│   │   │   ├── transform/   # Filter, map, aggregate, sort, pipeline
│   │   │   ├── schema/      # Type inference and validation
│   │   │   ├── parallel/    # Chunking and thread pool management
│   │   │   └── convert/     # Format conversion (CSV↔XLSX↔ODS)
│   │   └── ...
│   ├── dataforge-ffi/       # C ABI — opaque handles, #[repr(C)]
│   ├── dataforge-node/      # Node.js — napi-rs, async iterators
│   ├── dataforge-python/    # Python — PyO3, GIL release, NumPy
│   └── dataforge-wasm/      # WASM — wasm-bindgen, Web Workers
├── benches/                 # Criterion benchmarks
├── tests/                   # Integration tests & fixtures
└── examples/                # Usage examples for each language
```

### Key Design Principles

1. **All logic in `dataforge-core`**: Binding crates are thin wrappers only.
2. **Streaming by default**: No operation should load an entire file into memory.
3. **Memory-bounded**: Every operation respects configurable memory limits.
4. **Parallel where safe**: CPU-bound work is parallelized via Rayon.
5. **Zero-copy where possible**: Use `&str` slices into mmap'd regions.
6. **No panics across FFI**: All errors are caught and converted.

## Making Changes

### Branch Naming

- `feat/description` — New features
- `fix/description` — Bug fixes
- `perf/description` — Performance improvements
- `docs/description` — Documentation only
- `refactor/description` — Code restructuring
- `test/description` — Test additions/changes

### Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(core): add streaming ODS reader with SAX-style parsing
fix(csv): handle quoted fields with embedded newlines correctly
perf(xlsx): reduce memory allocation in shared string resolution
docs(readme): add Python binding usage examples
test(transform): add property-based tests for filter pipeline
```

## Coding Standards

### Rust

- **Format**: Always run `cargo fmt` before committing
- **Lints**: Code must pass `cargo clippy -- -D warnings`
- **Documentation**: All public items must have `///` doc comments
- **Error handling**: Use `thiserror` for error types, return `Result<T, DataForgeError>`
- **Unsafe**: Minimize `unsafe`. Every `unsafe` block must have a `// SAFETY:` comment
- **Tests**: Every public function should have at least one test

### Comments

Every module, struct, enum, and function should have descriptive comments explaining:
- **What** it does
- **Why** it exists (if not obvious)
- **How** it works (for complex algorithms)

```rust
/// Splits a memory-mapped CSV file into chunks at newline boundaries.
///
/// Each chunk is guaranteed to start and end at a complete row boundary,
/// making it safe to parse chunks independently in parallel threads.
///
/// # Algorithm
/// 1. Divide the file into N equal-sized regions (N = thread count)
/// 2. For each boundary, scan forward using `memchr` to find the next newline
/// 3. Adjust the boundary to the character after the newline
///
/// # Arguments
/// * `data` - Memory-mapped file content
/// * `num_chunks` - Number of chunks to split into
///
/// # Returns
/// Vector of `ChunkRange { offset, length }` describing each chunk
pub fn split_into_chunks(data: &[u8], num_chunks: usize) -> Vec<ChunkRange> {
    // ...
}
```

### Performance

- Prefer `SmallVec` over `Vec` for small, known-size collections
- Use `CompactString` for short strings (< 24 bytes)
- Pre-allocate buffers with `Vec::with_capacity()` when size is known
- Avoid `clone()` on hot paths — use references or `Cow<str>`
- Profile with `criterion` before and after optimization

## Testing

### Running Tests

```bash
# All tests
cargo test --workspace

# Specific crate
cargo test -p dataforge-core

# Specific test
cargo test csv_roundtrip

# With output
cargo test -- --nocapture

# Integration tests only
cargo test --workspace --test '*'
```

### Writing Tests

- Unit tests go in the same file, inside `#[cfg(test)] mod tests { ... }`
- Integration tests go in `tests/integration/`
- Test fixtures go in `tests/fixtures/`
- Use `tempfile` for tests that write files
- Test both success and error paths

### Test Categories

- **Unit tests**: Test individual functions in isolation
- **Integration tests**: Test full read/write/transform workflows
- **Property-based tests**: Use `proptest` for fuzzing-style tests
- **Benchmark tests**: Use `criterion` for performance regression detection
- **Memory tests**: Verify memory stays bounded during large file processing

## Submitting Changes

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes with clear, descriptive commits
4. Run the full test suite: `cargo test --workspace`
5. Run lints: `cargo clippy --workspace --all-targets -- -D warnings`
6. Run format: `cargo fmt --all`
7. Push your branch and open a Pull Request
8. Describe what your PR does and link any related issues
9. Wait for CI to pass and a maintainer to review

### PR Checklist

- [ ] Code compiles without warnings
- [ ] All existing tests pass
- [ ] New tests added for new functionality
- [ ] Public API changes are documented
- [ ] README updated if applicable
- [ ] Benchmarks run if performance-sensitive

## Release Process

1. Update version in all `Cargo.toml` files
2. Update `CHANGELOG.md`
3. Create a git tag: `git tag v0.x.y`
4. Push tag: `git push origin v0.x.y`
5. CI will automatically:
   - Build and test on all platforms
   - Publish to crates.io
   - Publish npm package
   - Publish PyPI package
   - Publish WASM package
   - Generate C headers and shared libraries

## Questions?

Open an issue on GitHub or start a discussion. We're happy to help!
