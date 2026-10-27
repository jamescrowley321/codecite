# Inspect loads this through the `inspect_ai` entry point, so the tasks run as
# `inspect eval codecite_evals/ab -T task_id=<id>` and, for the first task,
# `inspect eval codecite_evals/fastapi_middleware`.
from codecite_evals.task import ab, fastapi_middleware  # noqa: F401
