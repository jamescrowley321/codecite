# Warm batch rerun 1: base vs branch (#308)

Ranked citations identical for all 44 queries.

- base load average (1/5/15 min): 14.98 10.55 7.78 → 15.14 10.66 7.83 → 16.88 11.18 8.03 → 16.07 11.20 8.07 → 15.42 11.15 8.07 → 14.51 11.09 8.08 → 13.43 10.93 8.05 → 11.09 10.59 8.05
- branch load average (1/5/15 min): 15.14 10.66 7.83 → 16.88 11.18 8.03 → 16.07 11.20 8.07 → 15.42 11.15 8.07 → 14.51 11.09 8.08 → 13.43 10.93 8.05 → 11.09 10.59 8.05 → 12.46 11.06 8.31

## Search latency (ms)

| Corpus | Class | Mode | Base p50 | Branch p50 | Change | Base p95 | Branch p95 | Change |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fhir | behaviour | cli | 87.1 | 121.2 | +39.1% | 99.5 | 154.1 | +54.9% |
| fhir | behaviour | mcp | 173.8 | 161.4 | -7.1% | 218.7 | 170.8 | -21.9% |
| fhir | broad-term | cli | 120.6 | 237.0 | +96.4% | 160.1 | 272.1 | +70.0% |
| fhir | broad-term | mcp | 223.3 | 215.4 | -3.6% | 268.7 | 249.7 | -7.1% |
| fhir | exact-symbol | cli | 109.4 | 150.5 | +37.5% | 184.5 | 255.1 | +38.3% |
| fhir | exact-symbol | mcp | 181.3 | 161.1 | -11.1% | 452.9 | 297.4 | -34.3% |
| fhir | miss | cli | 3.4 | 5.4 | +60.2% | 4.5 | 6.4 | +42.2% |
| fhir | miss | mcp | 1.9 | 3.2 | +68.3% | 2.2 | 5.2 | +132.8% |
| fhir | path-filter | cli | 42.1 | 70.5 | +67.5% | 49.3 | 121.2 | +145.9% |
| fhir | path-filter | mcp | 66.7 | 95.4 | +43.0% | 92.0 | 105.1 | +14.2% |
| fhir | rare-term | cli | 4.7 | 6.3 | +34.2% | 5.4 | 6.7 | +24.4% |
| fhir | rare-term | mcp | 3.0 | 4.4 | +45.8% | 3.3 | 11.6 | +257.5% |
| keycloak | ambiguous-symbol | cli | 103.1 | 59.7 | -42.1% | 116.7 | 64.0 | -45.1% |
| keycloak | ambiguous-symbol | mcp | 72.9 | 61.1 | -16.2% | 74.5 | 92.3 | +24.0% |
| keycloak | behaviour | cli | 64.6 | 31.9 | -50.7% | 108.3 | 50.0 | -53.8% |
| keycloak | behaviour | mcp | 38.8 | 30.6 | -21.1% | 53.4 | 91.0 | +70.2% |
| keycloak | broad-term | cli | 30.5 | 16.8 | -44.9% | 36.5 | 20.1 | -44.9% |
| keycloak | broad-term | mcp | 19.5 | 14.9 | -23.8% | 23.1 | 18.3 | -20.6% |
| keycloak | exact-symbol | cli | 45.8 | 24.9 | -45.6% | 142.6 | 84.9 | -40.4% |
| keycloak | exact-symbol | mcp | 30.6 | 26.0 | -15.0% | 91.5 | 80.3 | -12.2% |
| keycloak | miss | cli | 4.1 | 3.8 | -6.5% | 5.3 | 4.2 | -20.4% |
| keycloak | miss | mcp | 0.7 | 1.0 | +33.6% | 1.3 | 1.5 | +18.0% |
| keycloak | path-filter | cli | 13.8 | 9.9 | -28.6% | 17.1 | 11.3 | -34.0% |
| keycloak | path-filter | mcp | 8.5 | 9.0 | +5.4% | 12.9 | 10.7 | -17.5% |
| keycloak | rare-term | cli | 10.8 | 6.3 | -41.3% | 12.2 | 7.2 | -40.9% |
| keycloak | rare-term | mcp | 4.6 | 3.2 | -29.0% | 5.5 | 5.2 | -5.6% |
| kubernetes | ambiguous-symbol | cli | 96.5 | 46.0 | -52.3% | 155.1 | 48.5 | -68.7% |
| kubernetes | ambiguous-symbol | mcp | 86.3 | 42.6 | -50.7% | 96.5 | 43.9 | -54.5% |
| kubernetes | behaviour | cli | 118.7 | 59.0 | -50.3% | 132.9 | 60.6 | -54.4% |
| kubernetes | behaviour | mcp | 97.2 | 56.6 | -41.8% | 144.5 | 57.0 | -60.6% |
| kubernetes | broad-term | cli | 68.2 | 33.2 | -51.3% | 178.1 | 35.1 | -80.3% |
| kubernetes | broad-term | mcp | 59.2 | 30.2 | -49.0% | 63.5 | 31.4 | -50.6% |
| kubernetes | exact-symbol | cli | 65.6 | 33.1 | -49.6% | 130.4 | 56.4 | -56.7% |
| kubernetes | exact-symbol | mcp | 62.0 | 30.5 | -50.8% | 97.0 | 49.9 | -48.5% |
| kubernetes | miss | cli | 3.9 | 3.5 | -11.8% | 8.3 | 5.1 | -38.2% |
| kubernetes | miss | mcp | 0.9 | 0.9 | -4.0% | 1.2 | 1.5 | +20.3% |
| kubernetes | path-filter | cli | 32.1 | 22.3 | -30.4% | 36.7 | 25.5 | -30.4% |
| kubernetes | path-filter | mcp | 26.7 | 20.2 | -24.7% | 29.6 | 21.8 | -26.5% |
| kubernetes | rare-term | cli | 6.2 | 5.1 | -18.8% | 7.6 | 6.2 | -18.2% |
| kubernetes | rare-term | mcp | 3.4 | 2.3 | -30.3% | 3.5 | 3.8 | +7.1% |
| linux | ambiguous-symbol | cli | 118.6 | 60.1 | -49.3% | 127.6 | 102.6 | -19.6% |
| linux | ambiguous-symbol | mcp | 114.2 | 71.1 | -37.7% | 226.7 | 103.4 | -54.4% |
| linux | behaviour | cli | 261.1 | 163.5 | -37.4% | 424.5 | 354.2 | -16.6% |
| linux | behaviour | mcp | 343.9 | 179.6 | -47.8% | 653.0 | 504.6 | -22.7% |
| linux | broad-term | cli | 162.5 | 74.4 | -54.2% | 173.2 | 104.5 | -39.6% |
| linux | broad-term | mcp | 158.8 | 88.0 | -44.6% | 286.9 | 109.8 | -61.7% |
| linux | exact-symbol | cli | 253.3 | 134.8 | -46.8% | 802.7 | 780.7 | -2.7% |
| linux | exact-symbol | mcp | 250.7 | 153.5 | -38.8% | 1129.9 | 816.2 | -27.8% |
| linux | miss | cli | 3.9 | 3.8 | -2.3% | 5.1 | 4.0 | -21.5% |
| linux | miss | mcp | 1.2 | 1.1 | -4.8% | 3.8 | 1.7 | -55.5% |
| linux | path-filter | cli | 80.3 | 83.1 | +3.6% | 84.0 | 99.0 | +17.8% |
| linux | path-filter | mcp | 77.9 | 78.3 | +0.5% | 137.0 | 158.6 | +15.8% |
| linux | rare-term | cli | 5.9 | 4.6 | -21.6% | 7.1 | 7.0 | -0.6% |
| linux | rare-term | mcp | 2.5 | 3.2 | +31.1% | 4.0 | 5.3 | +30.6% |

## Index

| Corpus | Base bytes | Branch bytes | Change | Base wall s | Branch wall s | Change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fhir | 802,443,264 | 802,254,848 | -0.0% | 85.1 | 90.1 | +5.9% |
| linux | 2,567,720,960 | 2,567,331,840 | -0.0% | 316.8 | 256.0 | -19.2% |
| kubernetes | 686,465,024 | 685,993,984 | -0.1% | 54.5 | 53.0 | -2.7% |
| keycloak | 255,045,632 | 254,574,592 | -0.2% | 19.8 | 19.1 | -3.5% |
