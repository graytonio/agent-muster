# The muster server image: muster, herdr, Claude Code, git, gh and tailscale,
# implementing the runtime contract in docs/RUNTIME.md. Built for linux/amd64
# and linux/arm64 by .github/workflows/release.yml from the release binaries.
#
# Build context is this directory. The muster binary for each platform must be
# staged at bin/<TARGETARCH>/muster before building, e.g.:
#   mkdir -p container/bin/arm64 && cp muster container/bin/arm64/
#   docker build -t muster container

FROM debian:bookworm-slim

ARG TARGETARCH

# herdr: prebuilt static binaries from its GitHub release, pinned by checksum.
ARG HERDR_VERSION=0.9.3
ARG HERDR_SHA256_AMD64=18a8dc65f1c2fa485884344356dea1cfd911c6f06cf46fa78e193f4087f4dba7
ARG HERDR_SHA256_ARM64=4de7aa3e25678812e92960de64f7c2aaa1bca1f0f80a3c5e559837e231e1f5c0

# Claude Code: the native binary, checked against the release manifest.
ARG CLAUDE_VERSION=2.1.326

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash-completion ca-certificates curl git gnupg jq less openssh-client \
      procps tini util-linux \
 && install -d -m 755 /etc/apt/keyrings \
 && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
      -o /etc/apt/keyrings/githubcli.gpg \
 && echo "deb [arch=${TARGETARCH} signed-by=/etc/apt/keyrings/githubcli.gpg] https://cli.github.com/packages stable main" \
      > /etc/apt/sources.list.d/github-cli.list \
 && curl -fsSL https://pkgs.tailscale.com/stable/debian/bookworm.noarmor.gpg \
      -o /etc/apt/keyrings/tailscale.gpg \
 && echo "deb [signed-by=/etc/apt/keyrings/tailscale.gpg] https://pkgs.tailscale.com/stable/debian bookworm main" \
      > /etc/apt/sources.list.d/tailscale.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends gh tailscale \
 && rm -rf /var/lib/apt/lists/*

RUN case "${TARGETARCH}" in \
      amd64) herdr_arch=x86_64;  herdr_sum=${HERDR_SHA256_AMD64}; claude_platform=linux-x64 ;; \
      arm64) herdr_arch=aarch64; herdr_sum=${HERDR_SHA256_ARM64}; claude_platform=linux-arm64 ;; \
      *) echo "unsupported TARGETARCH ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
 && curl -fsSL -o /usr/local/bin/herdr \
      "https://github.com/herdrdev/herdr/releases/download/v${HERDR_VERSION}/herdr-linux-${herdr_arch}" \
 && echo "${herdr_sum}  /usr/local/bin/herdr" | sha256sum -c - \
 && base=https://downloads.claude.ai/claude-code-releases/${CLAUDE_VERSION} \
 && claude_sum=$(curl -fsSL "${base}/manifest.json" | jq -er --arg p "${claude_platform}" '.platforms[$p].checksum') \
 && curl -fsSL -o /usr/local/bin/claude "${base}/${claude_platform}/claude" \
 && echo "${claude_sum}  /usr/local/bin/claude" | sha256sum -c - \
 && chmod 755 /usr/local/bin/herdr /usr/local/bin/claude \
 && herdr --version && claude --version

# Agents and muster run as this user; its home is on the persistent volume.
# The entrypoint runs as root only to start tailscaled (for Tailscale SSH).
RUN useradd --uid 1000 --user-group --shell /bin/bash \
      --home-dir /var/lib/muster/home --no-create-home muster

COPY bin/${TARGETARCH}/muster /usr/local/libexec/muster
COPY muster-wrapper.sh /usr/local/bin/muster
COPY entrypoint.sh /usr/local/bin/muster-entrypoint
COPY healthcheck.sh /usr/local/bin/muster-healthcheck
COPY profile.sh /etc/profile.d/muster.sh
RUN chmod 755 /usr/local/libexec/muster /usr/local/bin/muster \
      /usr/local/bin/muster-entrypoint /usr/local/bin/muster-healthcheck \
 && chmod 644 /etc/profile.d/muster.sh \
 && /usr/local/libexec/muster --version

ENV MUSTER_DATA=/var/lib/muster \
    MUSTER_CONFIG_DIR=/etc/muster \
    DISABLE_AUTOUPDATER=1

VOLUME /var/lib/muster
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/muster-entrypoint"]

LABEL org.opencontainers.image.source=https://github.com/graytonio/agent-muster \
      org.opencontainers.image.description="muster with herdr, Claude Code, git, gh and tailscale"
