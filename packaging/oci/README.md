# MCP image base

CI and release packaging use this directory's Dockerfile and the same Debian
`trixie-slim` base. The base comes from Docker's official publisher on ECR Public:
`public.ecr.aws/docker/library/debian:trixie-slim`.

Docker [publishes its official images on ECR Public](https://www.docker.com/blog/news-from-aws-reinvent-docker-official-images-on-amazon-ecr-public/).
AWS [supports anonymous pulls](https://docs.aws.amazon.com/AmazonECR/latest/public/public-gallery.html).
This avoids the Docker Hub anonymous pull quota that stopped CI before its
image startup and persistence smoke could run. No registry secret is needed.

The distribution and tag are unchanged. A registry probe on 2026-10-09 found
byte-identical Docker Hub and ECR Public image indexes and `linux/amd64`
manifests for the tag:

- Index: `sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f`
- AMD64: `sha256:918311b7b6c4c6f68b232ba516584925f6c78ad82b6fd534b98979df6438e483`

These record the measured image identity; the existing mutable tag policy still
allows Docker's publisher to update the base. Future tag updates require the
normal image smoke to pass.

The CI `Extended feature surfaces` job builds the image from its own binaries
and runs `scripts/ci/mcp-stdio-smoke.py`. The release `MCP Registry` workflow
uses the same Dockerfile and smoke with checksum-verified release binaries.
Both keep the unprivileged user and persistent `/data` volume. A failed pull,
build, startup or persistence check still fails the workflow.
