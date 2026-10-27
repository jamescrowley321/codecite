# Inspect loads this through the `inspect_ai` entry point, so each task runs as
# `inspect eval codecite_evals/<name>`.
from codecite_evals.spec_task import spec_questions  # noqa: F401
from codecite_evals.task import backchannel_logout, client_auth_uri, fastapi_middleware  # noqa: F401
