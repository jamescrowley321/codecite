import os

from fastapi import FastAPI, HTTPException
from fastapi_identity_model import CurrentUser, TokenValidationMiddleware

app = FastAPI(title="Items API")

# Every route needs a valid bearer token except /health (exact match).
app.add_middleware(
    TokenValidationMiddleware,
    discovery_url=os.environ["OIDC_DISCOVERY_URL"],
    audience=os.environ["OIDC_AUDIENCE"],
    excluded_paths=["/health"],
)

ITEMS = [
    {"id": 1, "name": "hammer"},
    {"id": 2, "name": "wrench"},
    {"id": 3, "name": "pliers"},
]


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.get("/items")
async def list_items():
    return ITEMS


@app.get("/items/{item_id}")
async def get_item(item_id: int):
    for item in ITEMS:
        if item["id"] == item_id:
            return item
    raise HTTPException(status_code=404, detail="Item not found")


def _claim(user: CurrentUser, claim_type: str) -> str | None:
    claim = user.find_first(claim_type)
    return claim.value if claim else None


@app.get("/me")
async def me(user: CurrentUser):
    return {"sub": _claim(user, "sub"), "client_id": _claim(user, "client_id")}
