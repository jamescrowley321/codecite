# History benchmark scorecard

| Repository | Class | Engine | Queries | Recall@1 | Recall@10 | MRR |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| blind-peer-review | history-title | codecite | 32 | 0.31 | 0.88 | 0.44 |
| blind-peer-review | history-title | grep | 32 | 0.12 | 0.59 | 0.28 |
| blind-peer-review | history-title | popular | 32 | 0.22 | 0.44 | 0.28 |
| blind-peer-review | history-issue | codecite | 4 | 0.75 | 0.75 | 0.75 |
| blind-peer-review | history-issue | grep | 4 | 0.00 | 0.50 | 0.09 |
| blind-peer-review | history-issue | popular | 4 | 0.50 | 0.50 | 0.50 |
| blind-peer-review | all | codecite | 36 | 0.36 | 0.86 | 0.47 |
| blind-peer-review | all | grep | 36 | 0.11 | 0.58 | 0.25 |
| blind-peer-review | all | popular | 36 | 0.25 | 0.44 | 0.30 |
| codecite | history-title | codecite | 27 | 0.22 | 0.67 | 0.33 |
| codecite | history-title | grep | 27 | 0.11 | 0.41 | 0.19 |
| codecite | history-title | popular | 27 | 0.30 | 0.85 | 0.39 |
| codecite | history-issue | codecite | 16 | 0.31 | 0.56 | 0.38 |
| codecite | history-issue | grep | 16 | 0.06 | 0.25 | 0.10 |
| codecite | history-issue | popular | 16 | 0.44 | 0.94 | 0.53 |
| codecite | all | codecite | 43 | 0.26 | 0.63 | 0.35 |
| codecite | all | grep | 43 | 0.09 | 0.35 | 0.16 |
| codecite | all | popular | 43 | 0.35 | 0.88 | 0.44 |
| identity-model | history-title | codecite | 37 | 0.19 | 0.78 | 0.38 |
| identity-model | history-title | grep | 37 | 0.08 | 0.57 | 0.23 |
| identity-model | history-title | popular | 37 | 0.19 | 0.57 | 0.26 |
| identity-model | history-issue | codecite | 11 | 0.18 | 0.55 | 0.29 |
| identity-model | history-issue | grep | 11 | 0.18 | 0.45 | 0.21 |
| identity-model | history-issue | popular | 11 | 0.55 | 0.64 | 0.56 |
| identity-model | all | codecite | 48 | 0.19 | 0.73 | 0.36 |
| identity-model | all | grep | 48 | 0.10 | 0.54 | 0.22 |
| identity-model | all | popular | 48 | 0.27 | 0.58 | 0.33 |
| identity-stack | history-title | codecite | 29 | 0.14 | 0.59 | 0.28 |
| identity-stack | history-title | grep | 29 | 0.03 | 0.41 | 0.13 |
| identity-stack | history-title | popular | 29 | 0.17 | 0.59 | 0.23 |
| identity-stack | history-issue | codecite | 12 | 0.25 | 0.58 | 0.35 |
| identity-stack | history-issue | grep | 12 | 0.08 | 0.33 | 0.13 |
| identity-stack | history-issue | popular | 12 | 0.33 | 0.50 | 0.35 |
| identity-stack | all | codecite | 41 | 0.17 | 0.59 | 0.30 |
| identity-stack | all | grep | 41 | 0.05 | 0.39 | 0.13 |
| identity-stack | all | popular | 41 | 0.22 | 0.56 | 0.27 |
| terraform-provider-descope | history-title | codecite | 30 | 0.33 | 0.83 | 0.50 |
| terraform-provider-descope | history-title | grep | 30 | 0.17 | 0.57 | 0.30 |
| terraform-provider-descope | history-title | popular | 30 | 0.33 | 0.77 | 0.44 |
| terraform-provider-descope | history-issue | codecite | 1 | 1.00 | 1.00 | 1.00 |
| terraform-provider-descope | history-issue | grep | 1 | 0.00 | 1.00 | 0.12 |
| terraform-provider-descope | history-issue | popular | 1 | 0.00 | 0.00 | 0.00 |
| terraform-provider-descope | all | codecite | 31 | 0.35 | 0.84 | 0.52 |
| terraform-provider-descope | all | grep | 31 | 0.16 | 0.58 | 0.30 |
| terraform-provider-descope | all | popular | 31 | 0.32 | 0.74 | 0.43 |

- blind-peer-review: chunks: 1389 written · elapsed: 0.25s
- codecite: chunks: 2244 written · elapsed: 0.43s
- identity-model: chunks: 10001 written · elapsed: 1.26s
- identity-stack: chunks: 4101 written · elapsed: 0.60s
- terraform-provider-descope: chunks: 3834 written · elapsed: 0.53s
