# History benchmark scorecard

| Repository | Class | Engine | Queries | Recall@1 | Recall@10 | MRR |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| blind-peer-review | history-title | codecite | 37 | 0.16 | 0.38 | 0.21 |
| blind-peer-review | history-title | grep | 37 | 0.22 | 0.35 | 0.26 |
| blind-peer-review | history-issue | codecite | 5 | 0.20 | 0.40 | 0.23 |
| blind-peer-review | history-issue | grep | 5 | 0.20 | 0.40 | 0.25 |
| codecite | history-title | codecite | 37 | 0.16 | 0.70 | 0.29 |
| codecite | history-title | grep | 37 | 0.35 | 0.81 | 0.47 |
| codecite | history-issue | codecite | 18 | 0.28 | 0.89 | 0.46 |
| codecite | history-issue | grep | 18 | 0.56 | 1.00 | 0.73 |
| identity-model | history-title | codecite | 38 | 0.24 | 0.79 | 0.43 |
| identity-model | history-title | grep | 38 | 0.26 | 0.84 | 0.44 |
| identity-model | history-issue | codecite | 12 | 0.50 | 0.92 | 0.67 |
| identity-model | history-issue | grep | 12 | 0.42 | 1.00 | 0.63 |
| identity-stack | history-title | codecite | 29 | 0.14 | 0.48 | 0.26 |
| identity-stack | history-title | grep | 29 | 0.03 | 0.41 | 0.13 |
| identity-stack | history-issue | codecite | 12 | 0.25 | 0.50 | 0.34 |
| identity-stack | history-issue | grep | 12 | 0.08 | 0.33 | 0.13 |
| terraform-provider-descope | history-title | codecite | 30 | 0.33 | 0.80 | 0.50 |
| terraform-provider-descope | history-title | grep | 30 | 0.17 | 0.57 | 0.30 |
| terraform-provider-descope | history-issue | codecite | 1 | 1.00 | 1.00 | 1.00 |
| terraform-provider-descope | history-issue | grep | 1 | 0.00 | 1.00 | 0.12 |

- blind-peer-review: chunks: 1650 written · elapsed: 1.54s
- codecite: chunks: 2244 written · elapsed: 1.66s
- identity-model: chunks: 10001 written · elapsed: 1.98s
- identity-stack: chunks: 4101 written · elapsed: 2.00s
- terraform-provider-descope: chunks: 3834 written · elapsed: 1.93s
