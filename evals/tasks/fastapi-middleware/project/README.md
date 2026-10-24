# Items API

A small FastAPI service.

    uv sync
    uv run uvicorn app.main:app --reload

Routes:

- `GET /health`: liveness check
- `GET /items`: every item
- `GET /items/{id}`: one item
- `GET /me`: the calling identity (not implemented yet)
