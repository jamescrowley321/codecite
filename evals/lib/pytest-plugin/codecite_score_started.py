"""Loaded by the score phase with `-p codecite_score_started`: marks that
pytest itself started, before any conftest (and so the agent's code) is
imported. A run with no JUnit report but this marker ended inside pytest."""

import os

_marker = os.environ.get("CODECITE_SCORE_STARTED")
if _marker:
    with open(_marker, "w") as f:
        f.write("started\n")
