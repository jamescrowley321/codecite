#!/usr/bin/env node
"use strict";

class Client {
  validate(token) {
    return token.length > 0;
  }
}

function helper() {
  return 1;
}

module.exports = { Client, helper };
