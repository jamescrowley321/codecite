from urllib.parse import parse_qs

from fastapi import FastAPI, Request, Response

from app import sessions
from app.logout import InvalidLogoutToken, decode_logout_token

app = FastAPI(title="Notes")

NOTES = [
    {"id": 1, "text": "buy milk"},
    {"id": 2, "text": "call the plumber"},
]


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.get("/notes")
async def list_notes():
    return NOTES


@app.post("/backchannel-logout")
async def backchannel_logout(request: Request):
    form = parse_qs((await request.body()).decode())
    token = form.get("logout_token", [None])[0]
    if not token:
        return Response(status_code=400)
    try:
        claims = await decode_logout_token(token)
    except InvalidLogoutToken:
        return Response(status_code=400)
    sessions.end(sub=claims.get("sub"), sid=claims.get("sid"))
    return Response(status_code=200, headers={"Cache-Control": "no-store"})
