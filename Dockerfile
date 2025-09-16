FROM rust:1.89 AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock .

RUN apt update && apt install -y protobuf-compiler
RUN cargo build --release || true

COPY . .

RUN cargo build --release

FROM debian:bookworm-slim

WORKDIR /app

RUN apt update && apt install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sharify-be ./sharify
COPY --from=builder /app/.env .
COPY --from=builder /app/*.pem .

EXPOSE 3100

CMD ["./sharify"]
