# VigilAI — deploys anywhere (VM / Docker / Kubernetes / on-prem).
# Multi-stage: build the static-ish Rust binary, then a slim runtime that
# carries `git` (the validation/act stages use git worktrees) and CA certs.
#
# Build context must include BOTH crates (the wrapper + its engine):
#   docker build -f vigil/Dockerfile -t vigil:latest .        # run from VigilAI/
#
# BYO engine: the deterministic pipeline (ingest→correlate→triage→validate)
# runs with --no-engine out of the box. To enable RCA, provide an engine —
# e.g. mount your logged-in Claude CLI: -v "$HOME/.claude:/root/.claude" and
# install the `claude` binary, or (when the API adapter lands) pass a key.

# Pin to bookworm so the build glibc matches the runtime base below.
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
# gcc/libc headers: libsqlite3-sys (bundled) compiles SQLite from C source.
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config gcc libc6-dev && rm -rf /var/lib/apt/lists/*
# Copy both crates preserving the ../vigil-engine path dependency layout.
COPY vigil-engine /build/vigil-engine
COPY vigil /build/vigil
WORKDIR /build/vigil
RUN cargo build --release && strip target/release/vigil

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates && rm -rf /var/lib/apt/lists/*
# Non-root by default; the data plane is a mounted volume.
RUN useradd -m -u 10001 vigil
COPY --from=builder /build/vigil/target/release/vigil /usr/local/bin/vigil
USER vigil
WORKDIR /var/lib/vigil
VOLUME ["/var/lib/vigil"]
ENTRYPOINT ["vigil"]
CMD ["--help"]
