# Warm batch rerun 2: base vs branch (#308)

Ranked citations identical for all 44 queries.

- base load average (1/5/15 min): 12.46 11.06 8.31 → 12.90 11.17 8.36 → 12.84 11.22 8.40 → 12.95 11.29 8.46 → 12.64 11.25 8.46 → 11.96 11.15 8.46 → 12.28 11.23 8.50 → 10.69 10.95 8.49
- branch load average (1/5/15 min): 12.90 11.17 8.36 → 12.84 11.22 8.40 → 12.95 11.29 8.46 → 12.64 11.25 8.46 → 11.96 11.15 8.46 → 12.28 11.23 8.50 → 10.69 10.95 8.49 → 9.77 10.73 8.47

## Search latency (ms)

| Corpus | Class | Mode | Base p50 | Branch p50 | Change | Base p95 | Branch p95 | Change |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fhir | behaviour | cli | 95.5 | 85.3 | -10.7% | 116.6 | 121.8 | +4.4% |
| fhir | behaviour | mcp | 182.6 | 87.6 | -52.0% | 202.6 | 115.5 | -43.0% |
| fhir | broad-term | cli | 146.5 | 156.7 | +6.9% | 174.8 | 170.1 | -2.7% |
| fhir | broad-term | mcp | 264.5 | 149.1 | -43.6% | 276.4 | 171.8 | -37.9% |
| fhir | exact-symbol | cli | 116.2 | 114.0 | -1.9% | 226.7 | 167.6 | -26.1% |
| fhir | exact-symbol | mcp | 194.2 | 110.9 | -42.9% | 319.4 | 209.0 | -34.6% |
| fhir | miss | cli | 3.8 | 4.3 | +12.9% | 5.5 | 6.2 | +13.8% |
| fhir | miss | mcp | 2.1 | 2.0 | -3.2% | 2.3 | 3.3 | +43.8% |
| fhir | path-filter | cli | 45.1 | 55.8 | +23.7% | 71.1 | 74.3 | +4.6% |
| fhir | path-filter | mcp | 78.4 | 61.0 | -22.2% | 83.0 | 64.6 | -22.2% |
| fhir | rare-term | cli | 4.3 | 5.6 | +31.1% | 6.1 | 7.0 | +13.6% |
| fhir | rare-term | mcp | 3.1 | 2.9 | -7.7% | 5.1 | 4.5 | -12.6% |
| keycloak | ambiguous-symbol | cli | 170.7 | 65.8 | -61.4% | 177.9 | 128.2 | -27.9% |
| keycloak | ambiguous-symbol | mcp | 130.9 | 55.1 | -57.9% | 182.5 | 58.2 | -68.1% |
| keycloak | behaviour | cli | 77.8 | 45.2 | -41.9% | 132.3 | 66.3 | -49.9% |
| keycloak | behaviour | mcp | 77.7 | 27.7 | -64.4% | 131.3 | 43.4 | -67.0% |
| keycloak | broad-term | cli | 38.0 | 17.1 | -55.0% | 53.0 | 24.7 | -53.3% |
| keycloak | broad-term | mcp | 49.4 | 12.4 | -74.8% | 51.5 | 13.5 | -73.7% |
| keycloak | exact-symbol | cli | 57.7 | 31.8 | -44.9% | 192.4 | 119.1 | -38.1% |
| keycloak | exact-symbol | mcp | 61.9 | 20.2 | -67.4% | 229.1 | 74.3 | -67.6% |
| keycloak | miss | cli | 5.3 | 3.3 | -37.8% | 5.8 | 4.3 | -25.7% |
| keycloak | miss | mcp | 1.4 | 0.7 | -48.7% | 2.2 | 1.3 | -40.7% |
| keycloak | path-filter | cli | 17.4 | 13.4 | -22.7% | 24.4 | 15.8 | -35.2% |
| keycloak | path-filter | mcp | 14.7 | 7.1 | -51.8% | 20.4 | 7.7 | -62.2% |
| keycloak | rare-term | cli | 15.0 | 6.2 | -58.4% | 15.4 | 8.7 | -43.8% |
| keycloak | rare-term | mcp | 8.4 | 2.5 | -69.8% | 11.7 | 4.5 | -61.8% |
| kubernetes | ambiguous-symbol | cli | 86.7 | 47.5 | -45.1% | 159.7 | 54.5 | -65.8% |
| kubernetes | ambiguous-symbol | mcp | 99.8 | 57.3 | -42.6% | 121.1 | 82.5 | -31.9% |
| kubernetes | behaviour | cli | 103.6 | 63.7 | -38.5% | 130.8 | 66.0 | -49.5% |
| kubernetes | behaviour | mcp | 123.4 | 77.1 | -37.5% | 181.5 | 94.6 | -47.9% |
| kubernetes | broad-term | cli | 68.4 | 36.0 | -47.4% | 87.8 | 42.0 | -52.2% |
| kubernetes | broad-term | mcp | 67.7 | 46.6 | -31.2% | 101.8 | 56.2 | -44.8% |
| kubernetes | exact-symbol | cli | 64.1 | 36.2 | -43.6% | 105.7 | 60.6 | -42.7% |
| kubernetes | exact-symbol | mcp | 68.7 | 39.3 | -42.8% | 117.4 | 69.7 | -40.7% |
| kubernetes | miss | cli | 3.3 | 3.4 | +2.6% | 5.0 | 3.6 | -28.1% |
| kubernetes | miss | mcp | 1.6 | 1.5 | -7.8% | 1.7 | 1.6 | -9.4% |
| kubernetes | path-filter | cli | 31.4 | 27.7 | -11.9% | 37.9 | 32.3 | -14.7% |
| kubernetes | path-filter | mcp | 36.8 | 29.6 | -19.6% | 49.1 | 34.2 | -30.4% |
| kubernetes | rare-term | cli | 6.2 | 5.1 | -18.0% | 7.6 | 6.1 | -19.2% |
| kubernetes | rare-term | mcp | 4.1 | 3.7 | -11.7% | 4.4 | 4.5 | +2.0% |
| linux | ambiguous-symbol | cli | 122.1 | 44.0 | -63.9% | 129.7 | 50.5 | -61.1% |
| linux | ambiguous-symbol | mcp | 114.6 | 42.6 | -62.8% | 119.3 | 46.3 | -61.2% |
| linux | behaviour | cli | 260.9 | 135.4 | -48.1% | 488.4 | 223.4 | -54.3% |
| linux | behaviour | mcp | 259.1 | 132.5 | -48.9% | 404.8 | 223.8 | -44.7% |
| linux | broad-term | cli | 160.3 | 59.9 | -62.6% | 176.7 | 63.6 | -64.0% |
| linux | broad-term | mcp | 155.5 | 55.0 | -64.6% | 159.1 | 56.4 | -64.6% |
| linux | exact-symbol | cli | 254.5 | 107.8 | -57.6% | 847.4 | 601.9 | -29.0% |
| linux | exact-symbol | mcp | 248.4 | 102.6 | -58.7% | 819.0 | 594.4 | -27.4% |
| linux | miss | cli | 3.4 | 3.7 | +8.2% | 4.1 | 4.8 | +15.8% |
| linux | miss | mcp | 0.9 | 1.1 | +23.1% | 1.6 | 1.9 | +20.6% |
| linux | path-filter | cli | 79.4 | 61.0 | -23.1% | 83.3 | 63.8 | -23.5% |
| linux | path-filter | mcp | 77.6 | 57.0 | -26.5% | 80.9 | 62.9 | -22.3% |
| linux | rare-term | cli | 6.4 | 5.1 | -19.8% | 7.4 | 5.9 | -19.7% |
| linux | rare-term | mcp | 3.6 | 3.0 | -16.5% | 4.2 | 4.1 | -1.3% |

## Index

| Corpus | Base bytes | Branch bytes | Change | Base wall s | Branch wall s | Change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fhir | 802,443,264 | 802,254,848 | -0.0% | 85.1 | 90.1 | +5.9% |
| linux | 2,567,720,960 | 2,567,331,840 | -0.0% | 316.8 | 256.0 | -19.2% |
| kubernetes | 686,465,024 | 685,993,984 | -0.1% | 54.5 | 53.0 | -2.7% |
| keycloak | 255,045,632 | 254,574,592 | -0.2% | 19.8 | 19.1 | -3.5% |
