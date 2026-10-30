# The sandbox every sample runs in: pi and Claude Code, the task's project with its
# dependencies installed, identity-model's source, the task's spec documents
# as files when it names a directory for them, and codecite with the `eval`
# corpus indexed. The container has no route to the internet but the
# compose file's proxy, which lets through only Claude Code's API host; pi's
# model calls leave through Inspect's agent bridge.
#
# A replay task's project is its repository at the base commit, in the
# task's language: prepare.sh sets LANGUAGE's default, which picks the stage
# the toolchain under /opt/toolchain comes from. Node and Python are in the
# base image already.
ARG LANGUAGE=none

FROM scratch AS toolchain-none
COPY pi-models.json /opt/toolchain/.none
FROM toolchain-none AS toolchain-python
FROM toolchain-none AS toolchain-node
FROM scratch AS toolchain-go
COPY --from=golang:1.27.1-trixie@sha256:8f58fd67ea075142d947a60e0caa4317746a55118d312f027793d382c7741734 /usr/local/go /opt/toolchain/go
FROM scratch AS toolchain-rust
COPY --from=rust:1.99.1-slim-trixie@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 /usr/local/cargo /opt/toolchain/cargo
COPY --from=rust:1.99.1-slim-trixie@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 /usr/local/rustup /opt/toolchain/rustup
FROM toolchain-${LANGUAGE} AS toolchain

FROM node:24-trixie-slim
ARG LANGUAGE

RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 git ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=ghcr.io/astral-sh/uv:0.12.5 /uv /uvx /usr/local/bin/
RUN npm install -g @earendil-works/pi-coding-agent@1.2.0 @anthropic-ai/claude-code@2.1.300

# Rust links with the system C compiler.
RUN if [ "$LANGUAGE" = rust ]; then \
      apt-get update \
      && apt-get install -y --no-install-recommends gcc libc6-dev \
      && rm -rf /var/lib/apt/lists/*; \
    fi
COPY --from=toolchain /opt/toolchain /opt/toolchain
ENV CARGO_HOME=/opt/toolchain/cargo RUSTUP_HOME=/opt/toolchain/rustup \
    PATH=/opt/toolchain/go/bin:/opt/toolchain/cargo/bin:/root/go/bin:$PATH

COPY identity-model /opt/identity-model
COPY project /work/project
WORKDIR /work/project
ENV UV_LINK_MODE=copy
# Install the project, and warm the hidden tests' tools so scoring runs
# offline. A task with no project (spec-questions) starts in an empty one. A
# replay task's project becomes a one-commit repository, so no later history
# reaches the agent, before warm.sh (prepare.sh) installs it.
COPY warm.sh /tmp/warm.sh
RUN if [ "$LANGUAGE" != none ]; then \
      git init -q . && git add -A \
      && git -c user.name=eval -c user.email=eval@localhost commit -qm export \
      && bash /tmp/warm.sh; \
    elif [ -f pyproject.toml ]; then \
      sed -i 's#@IDENTITY_MODEL@#/opt/identity-model#' pyproject.toml \
      && uv sync \
      && uv run --with pytest==9.1.1 --with httpx==0.28.1 python -c ''; \
    fi

# The `eval` corpus over identity-model, plus the task's spec documents when
# it names any (documents.toml, staged by prepare.sh; codecite fetches each
# at its pinned hash). codecite indexes a git source, so the export becomes a
# throwaway one-commit repo first. The documents specs.tsv lists are copied
# from codecite's cache to /opt/specs/<name>.html, so both arms read the very
# bytes codecite indexed; sha256sum checks each against its pin again. A
# replay task's corpus (corpus.toml, staged by prepare.sh) is its project's
# one commit instead.
COPY codecite /opt/codecite/bin/codecite
COPY COMMIT /tmp/COMMIT
COPY documents.toml specs.tsv corpus.toml /tmp/
RUN if [ -s /tmp/corpus.toml ]; then \
      sed -i "s/@COMMIT@/$(git -C /work/project rev-parse HEAD)/" /tmp/corpus.toml \
      && /opt/codecite/bin/codecite index eval --manifest /tmp/corpus.toml \
      && rm -rf /tmp/corpus.toml /tmp/documents.toml /tmp/specs.tsv /tmp/COMMIT /tmp/warm.sh /root/.cache/codecite; \
      exit; \
    fi \
 && git init -q /tmp/src && cp -r /opt/identity-model/. /tmp/src/ \
 && git -C /tmp/src add -A \
 && git -C /tmp/src -c user.name=eval -c user.email=eval@localhost commit -qm export \
 && printf 'name = "eval"\n\n[[source]]\nkind = "git"\nname = "identity-model"\nurl = "/tmp/src"\ncommit = "%s"\nlicense = "Apache-2.0"\nuse = "adapt"\n' "$(git -C /tmp/src rev-parse HEAD)" > /tmp/corpus.toml \
 && { echo; cat /tmp/documents.toml; } >> /tmp/corpus.toml \
 && /opt/codecite/bin/codecite index eval --manifest /tmp/corpus.toml \
 && while read -r name sha; do \
      mkdir -p /opt/specs \
      && cp "/root/.cache/codecite/documents/$sha" "/opt/specs/$name.html" \
      && echo "$sha  /opt/specs/$name.html" | sha256sum -c --quiet || exit 1; \
    done < /tmp/specs.tsv \
 && rm -rf /tmp/src /tmp/corpus.toml /tmp/documents.toml /tmp/specs.tsv /tmp/COMMIT /tmp/warm.sh /root/.cache/codecite

COPY pi-models.json /root/.pi/agent/models.json
ENV PI_OFFLINE=1 PI_TELEMETRY=0 PI_SKIP_VERSION_CHECK=1

CMD ["tail", "-f", "/dev/null"]
