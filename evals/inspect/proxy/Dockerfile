# The sandbox's only way out: an HTTPS proxy that lets CONNECT through to
# api.anthropic.com alone, for Claude Code (claude_code.py). Anything else,
# the spec sites included, it refuses.
FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends tinyproxy \
 && rm -rf /var/lib/apt/lists/*
COPY tinyproxy.conf /etc/tinyproxy/tinyproxy.conf
COPY filter /etc/tinyproxy/filter
EXPOSE 8888
CMD ["tinyproxy", "-d", "-c", "/etc/tinyproxy/tinyproxy.conf"]
