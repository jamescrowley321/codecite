# Answer a question about the identity specifications

The OAuth 2.0, OpenID Connect, JOSE and FAPI specifications are in
`{specs}`, one HTML file per document, named by the document
(`rfc6749.html`, `openid-connect-core-1_0.html`).

Question: {question}

Answer it from the specifications, and cite the one section that supports
your answer. Write both to `{answer_file}`, as exactly these two lines:

```
answer: <the answer>
section: <document>#section-<number>
```

The answer is a single word or value: `yes` or `no`, or the name or value the
question asks for (a parameter, claim, error code, header, scheme, path or
number), with no units and no explanation. The document is the file's name
without `.html`, and the number is the section's number as the specification
gives it, for example `rfc6749#section-2.3.1`.

Don't use the network. Stop once the file is written.
