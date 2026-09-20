# 多阶段构建：静态二进制，运行镜像 <20MB
FROM rust:slim AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/agent-manager-vault /usr/local/bin/agent-manager-vault
ENV VAULT_DB=/data/vault.sqlite3
EXPOSE 8787
CMD ["agent-manager-vault"]
