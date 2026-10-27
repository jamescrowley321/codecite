"""In-memory sessions, one per sign-in, keyed by the provider's session id."""

from dataclasses import dataclass


@dataclass(frozen=True)
class Session:
    sub: str
    sid: str


_sessions: dict[str, Session] = {}


def start(sub: str, sid: str) -> None:
    _sessions[sid] = Session(sub=sub, sid=sid)


def is_active(sid: str) -> bool:
    return sid in _sessions


def end(sub: str | None = None, sid: str | None = None) -> int:
    """End the session `sid`, or every session of `sub` when no sid is given.
    With both, end `sid` only if it belongs to `sub`. Returns how many ended."""
    if sid is not None:
        session = _sessions.get(sid)
        if session is None or (sub is not None and session.sub != sub):
            return 0
        del _sessions[sid]
        return 1
    if sub is None:
        return 0
    ended = [k for k, s in _sessions.items() if s.sub == sub]
    for k in ended:
        del _sessions[k]
    return len(ended)
