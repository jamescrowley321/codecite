# Warm A/B, alternating every call, 300 rounds

Each query runs on base and branch back to back; the side that goes first flips each round.

- linux load average (1/5/15 min): 1.25 2.22 3.59 → 1.29 2.20 3.57
- keycloak load average (1/5/15 min): 1.29 2.20 3.57 → 1.33 2.17 3.54
- kubernetes load average (1/5/15 min): 1.33 2.17 3.54 → 1.30 2.15 3.53
- fhir load average (1/5/15 min): 1.30 2.15 3.53 → 1.28 2.13 3.52

| Corpus | Class | Mode | Base p50 | Branch p50 | Change | Base p95 | Branch p95 | Change |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fhir | miss | cli | 3.1 | 3.0 | -0.3% | 4.0 | 4.0 | +0.6% |
| fhir | miss | mcp | 1.0 | 1.1 | +1.5% | 1.4 | 1.4 | +2.6% |
| fhir | rare-term | cli | 3.7 | 3.7 | +0.4% | 4.7 | 4.7 | -1.1% |
| fhir | rare-term | mcp | 1.7 | 1.7 | +0.8% | 2.2 | 2.3 | +4.5% |
| keycloak | miss | cli | 3.1 | 3.1 | -0.2% | 4.1 | 4.1 | +0.5% |
| keycloak | miss | mcp | 1.0 | 1.1 | +5.0% | 1.5 | 1.4 | -4.4% |
| keycloak | rare-term | cli | 8.0 | 5.5 | -31.6% | 9.7 | 6.8 | -29.7% |
| keycloak | rare-term | mcp | 5.4 | 3.4 | -37.4% | 7.3 | 4.6 | -37.2% |
| kubernetes | miss | cli | 3.0 | 3.0 | +1.8% | 3.8 | 4.0 | +3.5% |
| kubernetes | miss | mcp | 0.9 | 1.0 | +3.3% | 1.4 | 1.4 | +4.2% |
| kubernetes | rare-term | cli | 5.3 | 4.9 | -8.0% | 6.7 | 6.0 | -10.4% |
| kubernetes | rare-term | mcp | 2.9 | 2.7 | -4.3% | 3.8 | 3.7 | -3.3% |
| linux | miss | cli | 3.0 | 3.0 | +0.3% | 3.9 | 3.9 | -1.5% |
| linux | miss | mcp | 1.0 | 1.0 | -1.3% | 1.4 | 1.4 | +3.7% |
| linux | rare-term | cli | 5.0 | 4.6 | -9.4% | 6.3 | 5.6 | -10.3% |
| linux | rare-term | mcp | 3.1 | 2.5 | -19.3% | 4.4 | 3.6 | -18.2% |

Ranked citations identical between base and branch on every call.
