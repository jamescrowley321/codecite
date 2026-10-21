import { generateKeyPair, exportJWK } from "jose";
import Provider from "oidc-provider";

const PORT = parseInt(process.env.PORT || "9011", 10);
if (Number.isNaN(PORT) || PORT < 1 || PORT > 65535) {
  console.error(`Invalid PORT: ${process.env.PORT} (must be 1-65535)`);
  process.exit(1);
}
// Loopback by default; the container sets HOST=0.0.0.0 and compose publishes
// the port on 127.0.0.1 only.
const HOST = process.env.HOST || "127.0.0.1";
const ISSUER = process.env.ISSUER || `http://127.0.0.1:${PORT}`;

const API = "urn:codecite-eval:api";
const OTHER = "urn:codecite-eval:other";
const SECRET = "eval-secret";

const TTL_BY_CLIENT = { "eval-client": 600, "eval-expiring": 2 };
const DEFAULT_TTL = 600;

function client(client_id) {
  return {
    client_id,
    client_secret: SECRET,
    grant_types: ["client_credentials"],
    response_types: [],
    token_endpoint_auth_method: "client_secret_post",
  };
}

async function generateJwks() {
  const { privateKey } = await generateKeyPair("RS256", { extractable: true });
  return {
    keys: [
      {
        ...(await exportJWK(privateKey)),
        kid: "eval-rsa-key",
        use: "sig",
        alg: "RS256",
      },
    ],
  };
}

const provider = new Provider(ISSUER, {
  clients: [client("eval-client"), client("eval-expiring")],
  jwks: await generateJwks(),
  features: {
    clientCredentials: { enabled: true },
    devInteractions: { enabled: false },
    resourceIndicators: {
      enabled: true,
      defaultResource: () => API,
      getResourceServerInfo: (ctx, resourceIndicator, client) => {
        if (resourceIndicator !== API && resourceIndicator !== OTHER) {
          return undefined;
        }
        return {
          scope: "",
          accessTokenFormat: "jwt",
          accessTokenTTL: TTL_BY_CLIENT[client.clientId] ?? DEFAULT_TTL,
          jwt: { sign: { alg: "RS256" } },
        };
      },
      useGrantedResource: () => true,
    },
  },
  cookies: { keys: ["eval-fixture-cookie-key"] },
});

// Plain HTTP on loopback for a local fixture.
provider.proxy = true;

const server = provider.listen(PORT, HOST, () => {
  console.log(`eval OIDC fixture listening on ${ISSUER} (${HOST}:${PORT})`);
});

server.on("error", (err) => {
  console.error(`Listen failed: ${err.message}`);
  process.exit(1);
});

for (const signal of ["SIGTERM", "SIGINT"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
