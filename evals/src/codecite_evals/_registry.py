# Inspect loads this through the `inspect_ai` entry point, so each task runs as
# `inspect eval codecite_evals/<name>`.
from codecite_evals.task import client_auth_uri, fastapi_middleware  # noqa: F401
