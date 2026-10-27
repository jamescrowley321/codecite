# The sandbox every sample runs in: pi, the task's project with its
# dependencies installed, identity-model's source, the task's spec documents
# as files when it names a directory for them, and codecite with the `eval`
# corpus indexed. The container has no route to the internet; pi's
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
# Install the project, and warm the hidden tests' tools so scoring runs
# offline. A task with no project (spec-questions) starts in an empty one.
RUN if [ -f pyproject.toml ]; then \
      sed -i 's#@IDENTITY_MODEL@#/opt/identity-model#' pyproject.toml \
      && uv sync \
      && uv run --with pytest==9.1.1 --with httpx==0.28.1 python -c ''; \
    fi

# The `eval` corpus over identity-model, plus the task's spec documents when
# it names any (documents.toml, staged by prepare.sh; codecite fetches each
# at its pinned hash). codecite indexes a git source, so the export becomes a
# throwaway one-commit repo first. The documents specs.tsv lists are copied
# from codecite's cache to /opt/specs/<name>.html, so both arms read the very
# bytes codecite indexed; sha256sum checks each against its pin again.
COPY codecite /opt/codecite/bin/codecite
COPY COMMIT /tmp/COMMIT
COPY documents.toml specs.tsv /tmp/
RUN git init -q /tmp/src && cp -r /opt/identity-model/. /tmp/src/ \
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
 && rm -rf /tmp/src /tmp/corpus.toml /tmp/documents.toml /tmp/specs.tsv /tmp/COMMIT /root/.cache/codecite

COPY pi-models.json /root/.pi/agent/models.json
ENV PI_OFFLINE=1 PI_TELEMETRY=0 PI_SKIP_VERSION_CHECK=1

CMD ["tail", "-f", "/dev/null"]
