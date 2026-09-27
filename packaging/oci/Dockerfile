# The image the MCP Registry's Install button runs (aegis-gys8sx):
#
#   docker run -i --rm -v quipu-data:/data ghcr.io/scbrown/quipu:<version>
#
# It packages the RELEASE binaries rather than compiling. The image is then the
# exact bytes the release smoke-tested and checksummed, and it builds in seconds
# instead of repeating a 20-minute `--features full` compile on another runner.
#
# The build context is a directory holding `quipu` and `quipu-server` side by
# side (release.yml stages it from the release tarball; ci.yml from its own
# build). They must stay side by side: `quipu mcp` execs the sibling
# `quipu-server` and deliberately never searches PATH (src/cli_mcp.rs).
#
# linux/amd64 only, because that is the only release target today. Docker
# Desktop on Apple Silicon runs it under emulation. Add arm64 here once the
# release matrix ships it (aegis-oifbu9).

# trixie, not bookworm: the release binaries link glibc 2.35 (ubuntu-22.04) but
# CI's smoke build links whatever ubuntu-latest has (2.39). trixie's 2.41 runs
# both, so the image CI tests and the image we ship share one base. slim already
# carries the only non-libc dependency (libstdc++6).
FROM debian:trixie-slim

# The registry refuses to list the image unless this equals server.json `name`.
# scripts/ci/mcp-registry.py check keeps the two in step.
LABEL io.modelcontextprotocol.server.name="io.github.scbrown/quipu"
LABEL org.opencontainers.image.source="https://github.com/scbrown/quipu"
LABEL org.opencontainers.image.description="Quipu knowledge graph as a local MCP server over stdio"
LABEL org.opencontainers.image.licenses="MIT"

COPY --chmod=755 quipu quipu-server /usr/local/bin/

# Unprivileged, and /data owned by that user BEFORE a volume mounts there:
# Docker seeds a new named volume from the image directory, ownership included,
# so the first Install creates a volume this user can write to.
RUN useradd --system --uid 10001 --home-dir /data --no-create-home quipu \
    && mkdir /data && chown quipu:quipu /data
USER quipu
# quipu-server writes state relative to its working directory (measured: the
# verdict-signing key at .quipu/verifier.pk8). Working in /data keeps it in the
# volume; in the throwaway container layer every restart would mint a new key.
WORKDIR /data
ENV HOME=/data

# A fresh database per volume, created at first start: nothing here points at a
# server the user does not own.
ENTRYPOINT ["/usr/local/bin/quipu", "mcp", "--db", "/data/quipu.db"]
