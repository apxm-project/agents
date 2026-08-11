# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e
ARG SOURCE_DATE_EPOCH=1786322092
FROM --platform=linux/arm64 rust:1.89-bookworm@sha256:948f9b08a66e7fe01b03a98ef1c7568292e07ec2e4fe90d88c07bb14563c84ff AS build

ARG TARGETPLATFORM
ARG APXM_SOURCE_REVISION=140ce5b16b4bc4a078800b52fffbfc8004169ee1
ARG APXM_REVIEWED_CARRIER_REVISION=8f767541413d0905d638b288a797c5825ceb1c76
ARG SOURCE_DATE_EPOCH
ENV RUSTUP_TOOLCHAIN=1.89.0
ENV CARGO_INCREMENTAL=0
ENV SOURCE_DATE_EPOCH=1786322092
WORKDIR /workspace

RUN test "$TARGETPLATFORM" = "linux/arm64"
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY contracts ./contracts
RUN cargo test --locked -p apxm-cli --test reference_host_jsonl --release
RUN cargo build --locked -p apxm-cli --bin apxm-reference-host --release
RUN test "$(od -An -tx1 -N20 target/release/apxm-reference-host | tr -d ' \n')" = "7f454c460201010000000000000000000300b700"
RUN touch --date="@$SOURCE_DATE_EPOCH" target/release/apxm-reference-host

FROM --platform=linux/arm64 debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241
ARG APXM_SOURCE_REVISION=140ce5b16b4bc4a078800b52fffbfc8004169ee1
ARG APXM_REVIEWED_CARRIER_REVISION=8f767541413d0905d638b288a797c5825ceb1c76
ARG SOURCE_DATE_EPOCH
LABEL org.opencontainers.image.title="APXM reference host"
LABEL org.opencontainers.image.source="https://github.com/apxm-project/agents.git"
LABEL org.opencontainers.image.revision="$APXM_SOURCE_REVISION"
LABEL io.apxm.reviewed-carrier-revision="$APXM_REVIEWED_CARRIER_REVISION"
LABEL io.apxm.reference-host-release-manifest-digest="sha256:47a6ec05c6b33ce0f603e7160280ce5e0edb3d59449fedcc5153b76718eec581"
COPY --from=build /workspace/target/release/apxm-reference-host /usr/local/bin/apxm-reference-host
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/apxm-reference-host"]
