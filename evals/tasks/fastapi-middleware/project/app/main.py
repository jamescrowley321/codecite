from fastapi import FastAPI, HTTPException

app = FastAPI(title="Items API")

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


@app.get("/me")
async def me():
    raise HTTPException(status_code=501, detail="Not implemented")
