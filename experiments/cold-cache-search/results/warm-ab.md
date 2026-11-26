# Warm A/B, alternating every call, 30 rounds

Each query runs on base and branch back to back; the side that goes first flips each round.

- linux load average (1/5/15 min): 6.58 4.17 4.68 → 1.64 2.85 4.02
- keycloak load average (1/5/15 min): 1.64 2.85 4.02 → 1.71 2.71 3.92
- kubernetes load average (1/5/15 min): 1.71 2.71 3.92 → 1.58 2.51 3.79
- fhir load average (1/5/15 min): 1.58 2.51 3.79 → 1.43 2.33 3.66

| Corpus | Class | Mode | Base p50 | Branch p50 | Change | Base p95 | Branch p95 | Change |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fhir | behaviour | cli | 73.1 | 47.3 | -35.2% | 79.7 | 51.8 | -35.0% |
| fhir | behaviour | mcp | 71.0 | 44.9 | -36.7% | 76.1 | 47.7 | -37.3% |
| fhir | broad-term | cli | 104.2 | 72.1 | -30.8% | 107.9 | 75.4 | -30.2% |
| fhir | broad-term | mcp | 101.6 | 69.2 | -31.9% | 106.4 | 77.3 | -27.3% |
| fhir | exact-symbol | cli | 95.0 | 63.5 | -33.1% | 129.6 | 92.2 | -28.9% |
| fhir | exact-symbol | mcp | 93.1 | 61.3 | -34.2% | 123.5 | 88.8 | -28.1% |
| fhir | miss | cli | 3.3 | 3.9 | +17.1% | 5.0 | 4.7 | -5.4% |
| fhir | miss | mcp | 1.0 | 0.9 | -9.7% | 1.8 | 1.6 | -8.9% |
| fhir | path-filter | cli | 38.0 | 31.0 | -18.3% | 40.7 | 36.6 | -10.0% |
| fhir | path-filter | mcp | 33.5 | 26.4 | -21.3% | 37.8 | 30.2 | -20.1% |
| fhir | rare-term | cli | 4.4 | 4.3 | -1.1% | 5.8 | 5.5 | -5.3% |
| fhir | rare-term | mcp | 1.7 | 1.6 | -9.5% | 2.6 | 2.8 | +6.7% |
| keycloak | ambiguous-symbol | cli | 71.4 | 55.0 | -22.9% | 76.6 | 61.6 | -19.6% |
| keycloak | ambiguous-symbol | mcp | 69.1 | 51.9 | -24.9% | 74.3 | 55.8 | -24.9% |
| keycloak | behaviour | cli | 45.3 | 30.5 | -32.6% | 60.0 | 43.1 | -28.3% |
| keycloak | behaviour | mcp | 37.5 | 27.5 | -26.8% | 53.4 | 38.1 | -28.7% |
| keycloak | broad-term | cli | 22.7 | 14.4 | -36.4% | 27.6 | 18.6 | -32.4% |
| keycloak | broad-term | mcp | 18.5 | 11.3 | -38.7% | 25.8 | 13.4 | -48.3% |
| keycloak | exact-symbol | cli | 33.0 | 22.7 | -31.1% | 91.1 | 70.9 | -22.2% |
| keycloak | exact-symbol | mcp | 28.5 | 20.0 | -29.7% | 86.2 | 68.7 | -20.3% |
| keycloak | miss | cli | 3.3 | 3.2 | -1.5% | 4.7 | 4.7 | -1.0% |
| keycloak | miss | mcp | 0.9 | 0.9 | -3.5% | 1.8 | 1.8 | +1.1% |
| keycloak | path-filter | cli | 13.0 | 11.2 | -14.0% | 15.3 | 14.3 | -6.5% |
| keycloak | path-filter | mcp | 7.9 | 6.7 | -16.3% | 12.1 | 10.6 | -11.9% |
| keycloak | rare-term | cli | 8.5 | 5.5 | -35.4% | 10.2 | 7.7 | -24.6% |
| keycloak | rare-term | mcp | 4.2 | 2.6 | -39.0% | 5.9 | 4.9 | -17.5% |
| kubernetes | ambiguous-symbol | cli | 78.1 | 43.5 | -44.2% | 88.4 | 46.7 | -47.2% |
| kubernetes | ambiguous-symbol | mcp | 74.9 | 39.4 | -47.4% | 78.0 | 42.1 | -46.0% |
| kubernetes | behaviour | cli | 88.4 | 56.4 | -36.2% | 93.9 | 66.4 | -29.3% |
| kubernetes | behaviour | mcp | 85.3 | 52.3 | -38.7% | 89.7 | 54.3 | -39.5% |
| kubernetes | broad-term | cli | 56.8 | 32.5 | -42.8% | 65.1 | 37.1 | -43.1% |
| kubernetes | broad-term | mcp | 52.7 | 29.5 | -44.0% | 54.7 | 31.2 | -43.0% |
| kubernetes | exact-symbol | cli | 56.4 | 31.6 | -44.0% | 84.4 | 52.9 | -37.3% |
| kubernetes | exact-symbol | mcp | 53.0 | 28.9 | -45.5% | 79.1 | 48.6 | -38.5% |
| kubernetes | miss | cli | 3.6 | 3.9 | +6.8% | 4.6 | 5.6 | +23.4% |
| kubernetes | miss | mcp | 1.0 | 1.1 | +8.2% | 1.8 | 1.6 | -13.5% |
| kubernetes | path-filter | cli | 27.2 | 22.4 | -17.7% | 31.0 | 27.2 | -12.2% |
| kubernetes | path-filter | mcp | 23.3 | 19.0 | -18.2% | 25.5 | 22.4 | -12.2% |
| kubernetes | rare-term | cli | 5.6 | 5.0 | -10.7% | 7.6 | 6.2 | -18.1% |
| kubernetes | rare-term | mcp | 2.4 | 2.1 | -11.0% | 4.4 | 4.0 | -10.3% |
| linux | ambiguous-symbol | cli | 115.5 | 45.1 | -61.0% | 123.8 | 58.8 | -52.5% |
| linux | ambiguous-symbol | mcp | 111.5 | 41.8 | -62.5% | 114.2 | 44.3 | -61.2% |
| linux | behaviour | cli | 248.8 | 133.8 | -46.2% | 393.0 | 220.1 | -44.0% |
| linux | behaviour | mcp | 247.7 | 127.4 | -48.6% | 395.0 | 214.5 | -45.7% |
| linux | broad-term | cli | 152.7 | 58.7 | -61.6% | 165.9 | 69.8 | -57.9% |
| linux | broad-term | mcp | 150.4 | 56.1 | -62.7% | 171.0 | 64.5 | -62.2% |
| linux | exact-symbol | cli | 242.8 | 108.1 | -55.5% | 785.9 | 594.1 | -24.4% |
| linux | exact-symbol | mcp | 241.0 | 105.2 | -56.3% | 781.1 | 590.2 | -24.4% |
| linux | miss | cli | 3.5 | 3.7 | +6.8% | 4.6 | 4.8 | +3.8% |
| linux | miss | mcp | 1.0 | 1.0 | +2.6% | 1.8 | 1.9 | +3.5% |
| linux | path-filter | cli | 77.7 | 59.5 | -23.4% | 83.6 | 68.3 | -18.2% |
| linux | path-filter | mcp | 74.3 | 55.9 | -24.7% | 81.0 | 62.1 | -23.3% |
| linux | rare-term | cli | 6.1 | 5.4 | -11.2% | 7.2 | 7.1 | -0.2% |
| linux | rare-term | mcp | 2.4 | 2.6 | +5.5% | 4.2 | 4.4 | +3.0% |

Ranked citations identical between base and branch on every call.
