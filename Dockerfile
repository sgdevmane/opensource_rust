FROM rust:1.80 as builder

WORKDIR /usr/src/dataforge
COPY . .

RUN cargo build --release -p dataforge-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /usr/src/dataforge/target/release/dataforge-server /app/dataforge-server

EXPOSE 8080
CMD ["/app/dataforge-server"]
