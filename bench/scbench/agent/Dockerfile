# syntax=docker/dockerfile:1
#
# The rdm payload image for SlopCodeBench's rdm-equipped agent image.
# `rdm-measure scbench image build` builds it with the rdm checkout under test
# as the context, tags it `rdm-scbench-payload:<HEAD>` and labels it
# `org.rdm.scbench.rdm-sha=<HEAD>`. The agent template (docker.j2) copies
# /opt/rdm out of it with `COPY --from`, because SCBench builds the agent
# image from a context holding only the rendered Dockerfile. Needs BuildKit
# (the default from Docker 23) for the cache mounts and Dockerfile.dockerignore.

# The workspace's rust-version. bookworm's glibc is older than the SCBench
# base image's (trixie), so the dynamically linked binary runs there.
FROM rust:1.94-bookworm AS build
ARG RDM_SHA
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p rdm-cli --bin rdm && \
    install -D -m 0755 target/release/rdm /out/opt/rdm/bin/rdm
RUN test -n "$RDM_SHA" && \
    install -m 0755 bench/scbench/agent/claude-rdm.sh /out/opt/rdm/bin/claude-rdm && \
    install -m 0755 bench/scbench/agent/plan-root-setup.sh /out/opt/rdm/bin/rdm-plan-root-setup && \
    cp -R plugins/rdm /out/opt/rdm/plugin && \
    printf '%s\n' "$RDM_SHA" > /out/opt/rdm/RDM_SHA

FROM scratch
COPY --from=build /out/opt/rdm /opt/rdm
