"""Hidden tests for the backchannel-logout task. The agent never sees these."""

import pytest
from conftest import _DROP, EVENT, OTHER_KEY, logout_token

from app import sessions

# Back-Channel Logout 1.0 §2.8: success is 200; some frameworks send 204.
OK = (200, 204)


def post(client, token):
    return client.post("/backchannel-logout", data={"logout_token": token})


def test_valid_token_ends_session(client, session):
    sub, sid = session
    assert post(client, logout_token(sub=sub, sid=sid)).status_code in OK
    assert not sessions.is_active(sid)


def test_valid_token_with_sid_only(client, session):
    _, sid = session
    assert post(client, logout_token(sub=_DROP, sid=sid)).status_code in OK
    assert not sessions.is_active(sid)


def test_valid_token_with_sub_only(client, session):
    sub, sid = session
    assert post(client, logout_token(sub=sub, sid=_DROP)).status_code in OK
    assert not sessions.is_active(sid)


REJECTED = {
    "bad_signature": lambda sub, sid: logout_token(key=OTHER_KEY, sub=sub, sid=sid),
    "wrong_iss": lambda sub, sid: logout_token(iss="https://op.invalid", sub=sub, sid=sid),
    "wrong_aud": lambda sub, sid: logout_token(aud="another-client", sub=sub, sid=sid),
    "nonce": lambda sub, sid: logout_token(nonce="n-0S6_WzA2Mj", sub=sub, sid=sid),
    "events_missing": lambda sub, sid: logout_token(events=_DROP, sub=sub, sid=sid),
    "events_not_object": lambda sub, sid: logout_token(events=[EVENT], sub=sub, sid=sid),
    "events_without_logout_member": lambda sub, sid: logout_token(
        events={"http://schemas.openid.net/event/other": {}}, sub=sub, sid=sid
    ),
    "no_sub_or_sid": lambda sub, sid: logout_token(sub=_DROP, sid=_DROP),
    "iat_missing": lambda sub, sid: logout_token(iat=_DROP, sub=sub, sid=sid),
    "jti_missing": lambda sub, sid: logout_token(jti=_DROP, sub=sub, sid=sid),
}


@pytest.mark.parametrize("case", sorted(REJECTED))
def test_rejected_with_400_and_session_kept(client, session, case):
    sub, sid = session
    assert post(client, REJECTED[case](sub, sid)).status_code == 400
    assert sessions.is_active(sid)
