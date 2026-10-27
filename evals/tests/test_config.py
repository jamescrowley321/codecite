"""Every file the eval reads resolves from config, which names a missing task."""

import pytest

from codecite_evals import config


def test_task_dir_is_under_the_evals_dir():
    assert config.task_dir("fastapi-middleware") == config.EVALS_DIR / "tasks" / "fastapi-middleware"
    assert config.COMPOSE.is_file()


def test_a_missing_task_says_where_it_looked():
    with pytest.raises(FileNotFoundError, match="CODECITE_EVALS_DIR"):
        config.task_dir("no-such-task")


def test_a_task_without_its_own_compose_file_uses_the_shared_one():
    assert config.compose("fastapi-middleware") == config.COMPOSE


def test_a_task_with_its_own_compose_file_uses_it():
    path = config.compose("backchannel-logout")
    assert path == config.EVALS_DIR / "inspect" / "compose.backchannel-logout.yaml"
    assert path.is_file()
