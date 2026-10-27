# Inspect loads this through the `inspect_ai` entry point, so any task runs as
# `inspect eval codecite_evals/ab -T task_id=<id>`, and each named one as
# `inspect eval codecite_evals/<name>`.
from codecite_evals.task import ab, backchannel_logout, client_auth_uri, fastapi_middleware  # noqa: F401
