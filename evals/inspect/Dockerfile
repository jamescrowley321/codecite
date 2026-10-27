# The sandbox every sample runs in: pi, the task's project with its
# dependencies installed, identity-model's source, and codecite with the
# `eval` corpus indexed. The container has no route to the internet; pi's
# model calls leave through Inspect's agent bridge.
FROM node:24-trixie-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends python3 git ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=ghcr.io/astral-sh/uv:0.12.5 /uv /uvx /usr/local/bin/
RUN npm install -g @earendil-works/pi-coding-agent@1.2.0

COPY identity-model /opt/identity-model
COPY project /work/project
WORKDIR /work/project
ENV UV_LINK_MODE=copy
# Install the project, and warm the hidden tests' tools so scoring runs offline.
RUN sed -i 's#@IDENTITY_MODEL@#/opt/identity-model#' pyproject.toml \
 && uv sync \
 && uv run --with pytest==9.1.1 --with httpx==0.28.1 python -c ''

# The `eval` corpus over identity-model, plus the task's spec documents when
# it names any (documents.toml, staged by prepare.sh; codecite fetches each
# at its pinned hash). codecite indexes a git source, so the export becomes a
# throwaway one-commit repo first.
COPY codecite /opt/codecite/bin/codecite
COPY COMMIT /tmp/COMMIT
COPY documents.toml /tmp/documents.toml
RUN git init -q /tmp/src && cp -r /opt/identity-model/. /tmp/src/ \
 && git -C /tmp/src add -A \
 && git -C /tmp/src -c user.name=eval -c user.email=eval@localhost commit -qm export \
 && printf 'name = "eval"\n\n[[source]]\nkind = "git"\nname = "identity-model"\nurl = "/tmp/src"\ncommit = "%s"\nlicense = "Apache-2.0"\nuse = "adapt"\n' "$(git -C /tmp/src rev-parse HEAD)" > /tmp/corpus.toml \
 && { echo; cat /tmp/documents.toml; } >> /tmp/corpus.toml \
 && /opt/codecite/bin/codecite index eval --manifest /tmp/corpus.toml \
 && rm -rf /tmp/src /tmp/corpus.toml /tmp/documents.toml /tmp/COMMIT /root/.cache/codecite

COPY pi-models.json /root/.pi/agent/models.json
ENV PI_OFFLINE=1 PI_TELEMETRY=0 PI_SKIP_VERSION_CHECK=1

CMD ["tail", "-f", "/dev/null"]
