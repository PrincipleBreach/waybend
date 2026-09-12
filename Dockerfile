# syntax=docker/dockerfile:1.7
FROM rust:1.88-bookworm AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY ui ./ui
COPY waybend.example.yml ./waybend.example.yml
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release && \
    cp target/release/waybend /tmp/waybend
RUN mkdir -p /tmp/waybend-data && touch /tmp/waybend-data/.keep

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /tmp/waybend /usr/local/bin/waybend
COPY --from=builder /src/waybend.example.yml /etc/waybend/config.yml
COPY --chown=65532:65532 --from=builder /tmp/waybend-data/ /var/lib/waybend/
WORKDIR /var/lib/waybend
VOLUME ["/var/lib/waybend"]
EXPOSE 8080/tcp 5353/tcp 5353/udp
ENTRYPOINT ["/usr/local/bin/waybend"]
CMD ["serve", "--config", "/etc/waybend/config.yml"]
