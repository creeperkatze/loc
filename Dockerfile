FROM rust:1-slim AS builder

WORKDIR /app

    COPY Cargo.toml Cargo.lock ./
    RUN mkdir src && echo "fn main() {}" > src/main.rs
    RUN cargo build --release
    RUN rm -rf src

COPY src/ ./src/
RUN touch src/main.rs
RUN cargo build --release

FROM debian:bookworm-slim AS runner

WORKDIR /app

COPY --from=builder /app/target/release/loctopus ./loctopus

ENV PORT=3000
EXPOSE 3000

CMD ["./loctopus"]
