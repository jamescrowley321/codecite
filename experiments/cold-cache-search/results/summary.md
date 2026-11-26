# Cold-cache search: base vs branch (#308)

Ranked citations identical for all 44 queries.

- base load average (1/5/15 min): 2.48 4.27 6.49 → 10.15 5.68 5.71
- branch load average (1/5/15 min): 10.15 5.68 5.71 → 11.19 8.87 7.08

## Search latency (ms)

| Corpus | Class | Mode | Base p50 | Branch p50 | Change | Base p95 | Branch p95 | Change |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fhir | behaviour | cli | 85.0 | 66.3 | -22.1% | 90.7 | 112.3 | +23.8% |
| fhir | behaviour | cli-cold | 1972.0 | 813.1 | -58.8% | 2522.1 | 990.5 | -60.7% |
| fhir | behaviour | mcp | 77.7 | 65.2 | -16.1% | 78.1 | 92.3 | +18.2% |
| fhir | broad-term | cli | 115.5 | 102.3 | -11.4% | 118.1 | 168.2 | +42.5% |
| fhir | broad-term | cli-cold | 2216.4 | 933.6 | -57.9% | 2427.1 | 1152.5 | -52.5% |
| fhir | broad-term | mcp | 110.8 | 95.5 | -13.8% | 119.7 | 116.2 | -3.0% |
| fhir | exact-symbol | cli | 96.1 | 94.6 | -1.6% | 136.3 | 171.3 | +25.6% |
| fhir | exact-symbol | cli-cold | 2280.4 | 880.1 | -61.4% | 3318.0 | 1254.0 | -62.2% |
| fhir | exact-symbol | mcp | 94.7 | 83.2 | -12.2% | 141.9 | 125.7 | -11.4% |
| fhir | miss | cli | 4.7 | 3.4 | -27.5% | 4.7 | 4.3 | -10.1% |
| fhir | miss | cli-cold | 23.8 | 23.9 | +0.3% | 34.9 | 43.4 | +24.4% |
| fhir | miss | mcp | 0.9 | 1.2 | +28.1% | 1.3 | 1.9 | +41.3% |
| fhir | path-filter | cli | 38.5 | 38.5 | -0.1% | 39.6 | 68.7 | +73.7% |
| fhir | path-filter | cli-cold | 1309.9 | 803.5 | -38.7% | 1886.3 | 1024.7 | -45.7% |
| fhir | path-filter | mcp | 35.9 | 33.9 | -5.6% | 37.5 | 44.1 | +17.5% |
| fhir | rare-term | cli | 4.5 | 5.1 | +12.5% | 5.1 | 6.2 | +21.1% |
| fhir | rare-term | cli-cold | 26.2 | 28.3 | +8.0% | 29.7 | 48.5 | +63.4% |
| fhir | rare-term | mcp | 1.4 | 1.4 | +5.3% | 1.5 | 2.8 | +92.8% |
| keycloak | ambiguous-symbol | cli | 73.4 | 56.2 | -23.4% | 85.1 | 57.9 | -32.0% |
| keycloak | ambiguous-symbol | cli-cold | 1192.1 | 664.8 | -44.2% | 1405.8 | 873.7 | -37.9% |
| keycloak | ambiguous-symbol | mcp | 72.2 | 54.0 | -25.3% | 77.7 | 57.3 | -26.2% |
| keycloak | behaviour | cli | 42.3 | 31.9 | -24.6% | 57.9 | 41.9 | -27.8% |
| keycloak | behaviour | cli-cold | 881.6 | 472.1 | -46.4% | 1040.2 | 734.6 | -29.4% |
| keycloak | behaviour | mcp | 40.2 | 26.0 | -35.2% | 59.4 | 39.7 | -33.2% |
| keycloak | broad-term | cli | 24.9 | 15.3 | -38.5% | 27.8 | 17.3 | -37.9% |
| keycloak | broad-term | cli-cold | 625.7 | 263.6 | -57.9% | 817.2 | 419.8 | -48.6% |
| keycloak | broad-term | mcp | 18.7 | 11.6 | -38.3% | 20.6 | 12.7 | -38.1% |
| keycloak | exact-symbol | cli | 31.9 | 23.0 | -27.8% | 98.5 | 72.0 | -26.9% |
| keycloak | exact-symbol | cli-cold | 832.7 | 352.5 | -57.7% | 1172.4 | 686.9 | -41.4% |
| keycloak | exact-symbol | mcp | 28.5 | 19.1 | -32.9% | 96.5 | 70.7 | -26.8% |
| keycloak | miss | cli | 4.0 | 2.9 | -25.9% | 4.5 | 3.3 | -27.2% |
| keycloak | miss | cli-cold | 20.6 | 22.1 | +7.0% | 21.4 | 30.5 | +42.2% |
| keycloak | miss | mcp | 1.1 | 0.6 | -44.0% | 1.6 | 0.8 | -51.9% |
| keycloak | path-filter | cli | 12.7 | 10.6 | -16.3% | 15.7 | 12.1 | -23.0% |
| keycloak | path-filter | cli-cold | 381.0 | 259.9 | -31.8% | 385.7 | 289.9 | -24.8% |
| keycloak | path-filter | mcp | 8.7 | 6.2 | -28.6% | 9.6 | 6.7 | -30.2% |
| keycloak | rare-term | cli | 8.2 | 6.0 | -26.7% | 8.8 | 6.2 | -28.8% |
| keycloak | rare-term | cli-cold | 84.3 | 35.2 | -58.3% | 85.7 | 38.1 | -55.6% |
| keycloak | rare-term | mcp | 5.0 | 2.3 | -53.8% | 6.4 | 2.5 | -61.1% |
| kubernetes | ambiguous-symbol | cli | 82.3 | 50.9 | -38.1% | 87.4 | 57.5 | -34.3% |
| kubernetes | ambiguous-symbol | cli-cold | 2269.9 | 1225.1 | -46.0% | 2272.4 | 1284.8 | -43.5% |
| kubernetes | ambiguous-symbol | mcp | 78.2 | 49.3 | -36.9% | 83.8 | 74.1 | -11.6% |
| kubernetes | behaviour | cli | 92.1 | 63.1 | -31.4% | 97.9 | 64.1 | -34.5% |
| kubernetes | behaviour | cli-cold | 1994.4 | 830.6 | -58.4% | 2013.5 | 860.1 | -57.3% |
| kubernetes | behaviour | mcp | 90.1 | 66.8 | -25.9% | 91.9 | 88.8 | -3.4% |
| kubernetes | broad-term | cli | 58.4 | 39.5 | -32.4% | 62.8 | 52.3 | -16.7% |
| kubernetes | broad-term | cli-cold | 1417.5 | 709.9 | -49.9% | 1421.1 | 805.1 | -43.3% |
| kubernetes | broad-term | mcp | 56.1 | 44.2 | -21.2% | 57.4 | 50.1 | -12.7% |
| kubernetes | exact-symbol | cli | 57.2 | 35.7 | -37.6% | 87.2 | 61.9 | -29.0% |
| kubernetes | exact-symbol | cli-cold | 1412.8 | 561.2 | -60.3% | 1864.2 | 988.3 | -47.0% |
| kubernetes | exact-symbol | mcp | 56.2 | 34.7 | -38.3% | 86.0 | 83.9 | -2.5% |
| kubernetes | miss | cli | 3.7 | 3.1 | -16.3% | 4.6 | 4.5 | -1.0% |
| kubernetes | miss | cli-cold | 20.7 | 22.8 | +10.4% | 21.9 | 24.1 | +10.1% |
| kubernetes | miss | mcp | 0.8 | 1.5 | +79.3% | 1.7 | 1.9 | +12.9% |
| kubernetes | path-filter | cli | 28.8 | 27.1 | -5.9% | 29.5 | 29.5 | -0.2% |
| kubernetes | path-filter | cli-cold | 963.5 | 671.2 | -30.3% | 965.1 | 858.4 | -11.0% |
| kubernetes | path-filter | mcp | 24.8 | 28.7 | +15.6% | 28.4 | 34.5 | +21.6% |
| kubernetes | rare-term | cli | 4.2 | 5.6 | +33.5% | 4.8 | 6.0 | +24.8% |
| kubernetes | rare-term | cli-cold | 34.0 | 40.2 | +18.1% | 37.7 | 49.8 | +32.3% |
| kubernetes | rare-term | mcp | 2.3 | 3.9 | +69.2% | 2.8 | 3.9 | +43.4% |
| linux | ambiguous-symbol | cli | 122.1 | 47.1 | -61.5% | 123.4 | 52.7 | -57.3% |
| linux | ambiguous-symbol | cli-cold | 5208.2 | 1946.3 | -62.6% | 5902.7 | 2135.9 | -63.8% |
| linux | ambiguous-symbol | mcp | 117.2 | 42.0 | -64.2% | 119.2 | 43.8 | -63.3% |
| linux | behaviour | cli | 248.2 | 138.8 | -44.1% | 393.1 | 220.9 | -43.8% |
| linux | behaviour | cli-cold | 7747.6 | 3200.1 | -58.7% | 10254.2 | 4504.0 | -56.1% |
| linux | behaviour | mcp | 245.5 | 132.6 | -46.0% | 394.8 | 239.1 | -39.5% |
| linux | broad-term | cli | 154.2 | 60.5 | -60.8% | 172.2 | 67.4 | -60.9% |
| linux | broad-term | cli-cold | 6517.5 | 2294.1 | -64.8% | 6857.2 | 2333.7 | -66.0% |
| linux | broad-term | mcp | 150.8 | 56.3 | -62.6% | 153.1 | 58.6 | -61.7% |
| linux | exact-symbol | cli | 243.7 | 106.3 | -56.4% | 778.4 | 643.7 | -17.3% |
| linux | exact-symbol | cli-cold | 8933.6 | 3236.1 | -63.8% | 11232.1 | 5369.8 | -52.2% |
| linux | exact-symbol | mcp | 240.9 | 109.1 | -54.7% | 789.8 | 626.3 | -20.7% |
| linux | miss | cli | 3.8 | 2.6 | -31.2% | 4.2 | 3.8 | -8.5% |
| linux | miss | cli-cold | 25.4 | 23.1 | -9.2% | 31.8 | 25.5 | -20.0% |
| linux | miss | mcp | 1.0 | 1.3 | +35.5% | 1.9 | 1.6 | -13.6% |
| linux | path-filter | cli | 78.6 | 59.3 | -24.5% | 83.3 | 66.3 | -20.3% |
| linux | path-filter | cli-cold | 4244.1 | 2905.4 | -31.5% | 4750.8 | 3232.7 | -32.0% |
| linux | path-filter | mcp | 76.5 | 56.0 | -26.9% | 81.6 | 56.7 | -30.5% |
| linux | rare-term | cli | 6.6 | 5.8 | -13.4% | 7.6 | 6.7 | -11.9% |
| linux | rare-term | cli-cold | 53.4 | 40.2 | -24.7% | 54.6 | 42.7 | -21.8% |
| linux | rare-term | mcp | 2.6 | 4.0 | +52.2% | 3.6 | 11.8 | +225.3% |

## Index

| Corpus | Base bytes | Branch bytes | Change | Base wall s | Branch wall s | Change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| fhir | 802,443,264 | 802,254,848 | -0.0% | 85.1 | 90.1 | +5.9% |
| linux | 2,567,720,960 | 2,567,331,840 | -0.0% | 316.8 | 256.0 | -19.2% |
| kubernetes | 686,465,024 | 685,993,984 | -0.1% | 54.5 | 53.0 | -2.7% |
| keycloak | 255,045,632 | 254,574,592 | -0.2% | 19.8 | 19.1 | -3.5% |
