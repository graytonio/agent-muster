# The muster server image: muster, herdr, Claude Code, git, gh, tailscale and rustup,
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

# rustup, pinned by checksum. Toolchains are not in the image: they install
# into rustup's default home (~/.rustup under the muster user's home, on the
# persistent volume), at startup for each cloned repo with a rust-toolchain
# file, or on first use. The default is the same in every context, including
# SSH commands that get no pod environment.
ARG RUSTUP_VERSION=1.29.3
ARG RUSTUP_SHA256_AMD64=b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55
ARG RUSTUP_SHA256_ARM64=9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash-completion build-essential ca-certificates curl git gnupg jq less \
      openssh-client pkg-config procps tini util-linux \
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
      amd64) herdr_arch=x86_64;  herdr_sum=${HERDR_SHA256_AMD64}; claude_platform=linux-x64 \
             rustup_triple=x86_64-unknown-linux-gnu;  rustup_sum=${RUSTUP_SHA256_AMD64} ;; \
      arm64) herdr_arch=aarch64; herdr_sum=${HERDR_SHA256_ARM64}; claude_platform=linux-arm64 \
             rustup_triple=aarch64-unknown-linux-gnu; rustup_sum=${RUSTUP_SHA256_ARM64} ;; \
      *) echo "unsupported TARGETARCH ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
 && curl -fsSL -o /usr/local/bin/rustup \
      "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${rustup_triple}/rustup-init" \
 && echo "${rustup_sum}  /usr/local/bin/rustup" | sha256sum -c - \
 && chmod 755 /usr/local/bin/rustup \
 && for proxy in cargo rustc rustdoc rustfmt cargo-fmt cargo-clippy clippy-driver \
                 rust-analyzer rust-gdb rust-lldb; do \
      ln -s rustup "/usr/local/bin/${proxy}"; \
    done \
 && rustup --version \
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
COPY env-wrapper.sh /usr/local/bin/muster
COPY env-wrapper.sh /usr/local/bin/gh
COPY entrypoint.sh /usr/local/bin/muster-entrypoint
COPY healthcheck.sh /usr/local/bin/muster-healthcheck
COPY profile.sh /etc/profile.d/muster.sh
RUN chmod 755 /usr/local/libexec/muster /usr/local/bin/muster /usr/local/bin/gh \
      /usr/local/bin/muster-entrypoint /usr/local/bin/muster-healthcheck \
 && chmod 644 /etc/profile.d/muster.sh \
 && /usr/local/libexec/muster --version

ENV MUSTER_DATA=/var/lib/muster \
    MUSTER_CONFIG_DIR=/etc/muster \
    DISABLE_AUTOUPDATER=1 \
    PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/var/lib/muster/home/.cargo/bin

VOLUME /var/lib/muster
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/muster-entrypoint"]

LABEL org.opencontainers.image.source=https://github.com/graytonio/agent-muster \
      org.opencontainers.image.description="muster with herdr, Claude Code, git, gh and tailscale"
