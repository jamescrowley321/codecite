# Warm A/B, fhir, 80 alternating rounds

Load average (1/5/15 min): 7.05 9.84 8.30 → 6.76 9.68 8.27

| Class | Side | p50 ms | p95 ms | min ms |
| --- | --- | ---: | ---: | ---: |
| miss | base | 3.3 | 4.4 | 2.4 |
| miss | branch | 3.1 | 4.3 | 1.9 |
| path-filter | base | 39.5 | 45.4 | 34.3 |
| path-filter | branch | 31.3 | 34.5 | 27.7 |
| rare-term | base | 5.6 | 7.4 | 3.6 |
| rare-term | branch | 4.3 | 6.3 | 2.9 |
