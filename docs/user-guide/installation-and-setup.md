# Installation and Setup (Docker on Debian Bookworm Slim)

This guide shows a container-first setup for DireClaw using a `debian:bookworm-slim` base image.

It covers:

- Installing runtime prerequisites
- Downloading the latest DireClaw release for detected Linux architecture
- Running the service in a container
- Authenticating Codex locally, then syncing OAuth artifacts into the container via `direclaw auth sync`

## Prerequisites

- Docker
- A GitHub network path from the image build environment
- A 1Password account/service token for auth sync (`OP_SERVICE_ACCOUNT_TOKEN`)
- A local machine where you can run Codex login interactively

## Example Dockerfile

```dockerfile
FROM debian:bookworm-slim

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ARG DEBIAN_FRONTEND=noninteractive

# Base tools:
# - curl/tar/ca-certificates: release download + extraction
# - git/openssh-client: common workflow dependencies
# - jq: robust JSON parsing for GitHub release metadata
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      git \
      jq \
      openssh-client \
      tar; \
    rm -rf /var/lib/apt/lists/*

# Optional but recommended for Codex provider execution:
# install Node.js + npm and Codex CLI.
# Replace with your standard Codex installation method if different.
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends nodejs npm; \
    npm install -g @openai/codex; \
    rm -rf /var/lib/apt/lists/*

# Install 1Password CLI (op), required by `direclaw auth sync`.
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "${arch}" in \
      amd64) op_arch="amd64" ;; \
      arm64) op_arch="arm64" ;; \
      *) echo "unsupported architecture for op: ${arch}" >&2; exit 1 ;; \
    esac; \
    curl -fsSLo /tmp/op.zip "https://cache.agilebits.com/dist/1P/op2/pkg/latest/op_linux_${op_arch}.zip"; \
    apt-get update; \
    apt-get install -y --no-install-recommends unzip; \
    unzip -d /tmp /tmp/op.zip; \
    install -m 0755 /tmp/op /usr/local/bin/op; \
    rm -rf /tmp/op /tmp/op.zip /var/lib/apt/lists/*

# Install latest DireClaw release matching Linux architecture.
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "${arch}" in \
      amd64) target="x86_64-unknown-linux-musl" ;; \
      arm64) target="aarch64-unknown-linux-musl" ;; \
      *) echo "unsupported architecture for DireClaw: ${arch}" >&2; exit 1 ;; \
    esac; \
    release_json="$(curl -fsSL https://api.github.com/repos/dmuso/direclaw/releases/latest)"; \
    tag="$(printf '%s' "${release_json}" | jq -r '.tag_name')"; \
    test -n "${tag}" && test "${tag}" != "null"; \
    asset="direclaw-${tag}-${target}.tar.gz"; \
    base_url="https://github.com/dmuso/direclaw/releases/download/${tag}"; \
    curl -fsSLo "/tmp/${asset}" "${base_url}/${asset}"; \
    curl -fsSLo /tmp/checksums.txt "${base_url}/checksums.txt"; \
    (cd /tmp && grep "  ${asset}\$" checksums.txt | sha256sum -c -); \
    tar -xzf "/tmp/${asset}" -C /tmp; \
    install -m 0755 /tmp/direclaw /usr/local/bin/direclaw; \
    rm -f "/tmp/${asset}" /tmp/checksums.txt /tmp/direclaw

# Create non-root runtime user.
RUN set -eux; \
    useradd --create-home --home-dir /home/direclaw --shell /bin/bash direclaw; \
    mkdir -p /home/direclaw/.direclaw /home/direclaw/.codex; \
    chown -R direclaw:direclaw /home/direclaw

USER direclaw
WORKDIR /home/direclaw

# Persist config/state and synced auth artifacts
VOLUME ["/home/direclaw/.direclaw", "/home/direclaw/.codex"]

# If auth_sync is enabled in /home/direclaw/.direclaw/config.yaml,
# start will run sync before workers become running.
CMD ["direclaw", "start"]
```

## Build and run

Build:

```bash
docker build -t direclaw:bookworm .
```

Run:

```bash
docker run --rm -it \
  -e OP_SERVICE_ACCOUNT_TOKEN="ops_..." \
  -e SLACK_APP_TOKEN="xapp-..." \
  -e SLACK_BOT_TOKEN="xoxb-..." \
  -v "$HOME/.direclaw:/home/direclaw/.direclaw" \
  -v "$HOME/.codex:/home/direclaw/.codex" \
  direclaw:bookworm
```

## Codex local auth -> container OAuth sync flow

Today, DireClaw supports auth sync using `onepassword` backend via `direclaw auth sync`.

### 1. Authenticate Codex locally

On your local host (not in container):

```bash
codex login
```

Confirm Codex auth file exists (typical path):

```bash
ls -l ~/.codex/auth.json
```

### 2. Store the local auth artifact in 1Password

Create a secure document/item in 1Password containing the contents of `~/.codex/auth.json`.
Use a secret reference such as:

```text
op://DireClaw/codex-auth/document
```

### 3. Configure DireClaw auth sync destination in container

In `~/.direclaw/config.yaml`:

```yaml
auth_sync:
  enabled: true
  sources:
    codex:
      backend: onepassword
      reference: op://DireClaw/codex-auth/document
      destination: "~/.codex/auth.json"
      owner_only: true
```

When this config is mounted to `/home/direclaw/.direclaw/config.yaml`, destination resolves to `/home/direclaw/.codex/auth.json` in the container.

### 4. Test sync explicitly

```bash
docker run --rm -it \
  -e OP_SERVICE_ACCOUNT_TOKEN="ops_..." \
  -v "$HOME/.direclaw:/home/direclaw/.direclaw" \
  -v "$HOME/.codex:/home/direclaw/.codex" \
  direclaw:bookworm \
  direclaw auth sync
```

Expected output includes:

- `auth sync complete`
- `sources=codex`

### 5. Start the service

```bash
docker run --rm -it \
  -e OP_SERVICE_ACCOUNT_TOKEN="ops_..." \
  -e SLACK_APP_TOKEN="xapp-..." \
  -e SLACK_BOT_TOKEN="xoxb-..." \
  -v "$HOME/.direclaw:/home/direclaw/.direclaw" \
  -v "$HOME/.codex:/home/direclaw/.codex" \
  direclaw:bookworm \
  direclaw start
```

If `auth_sync.enabled=true`, startup runs sync first.

## Related guides

- [User Guide Index](README.md)
- [Provider Auth Sync (1Password)](provider-auth-sync-1password.md)
- [Slack Setup](slack-setup.md)
- [Operator Runbook](operator-runbook.md)
