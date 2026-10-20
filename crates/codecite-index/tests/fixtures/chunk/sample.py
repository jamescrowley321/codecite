#!/usr/bin/env python3
"""A sample module."""

import os


class Client:
    def __init__(self, issuer):
        self.issuer = issuer

    @staticmethod
    def validate(token):
        return bool(token)


def helper():
    def inner():
        return 1

    return inner()
