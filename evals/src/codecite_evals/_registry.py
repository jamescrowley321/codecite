# Inspect loads this through the `inspect_ai` entry point, so the tasks run as
# `inspect eval codecite_evals/<task>`, e.g. codecite_evals/fastapi_middleware.
from codecite_evals.task import backchannel_logout, fastapi_middleware  # noqa: F401
