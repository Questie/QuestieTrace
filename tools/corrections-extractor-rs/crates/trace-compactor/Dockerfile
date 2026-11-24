# The compactor as an image. The build context is tools/corrections-extractor-rs, taken from
# a commit so that nothing uncommitted and no build directory goes into it:
#
#   git archive HEAD:tools/corrections-extractor-rs | docker build -f crates/trace-compactor/Dockerfile -t trace-compactor:$(git rev-parse --short HEAD) -
FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p trace-compactor

FROM debian:bookworm-slim
COPY --from=build /src/target/release/trace-compactor /usr/local/bin/trace-compactor
# The certificate roots are compiled in, so the image needs nothing else to reach the bucket.
USER 10002:10002
ENTRYPOINT ["/usr/local/bin/trace-compactor"]
