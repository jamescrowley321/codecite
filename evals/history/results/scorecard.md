# History benchmark scorecard

| Repository | Class | Engine | Queries | Recall@1 | Recall@10 | MRR |
| --- | --- | --- | ---: | ---: | ---: | ---: |
| blind-peer-review | history-title | codecite | 32 | 0.31 | 0.88 | 0.44 |
| blind-peer-review | history-title | grep | 32 | 0.12 | 0.59 | 0.28 |
| blind-peer-review | history-title | popular | 32 | 0.22 | 0.44 | 0.28 |
| blind-peer-review | history-issue | codecite | 4 | 0.50 | 1.00 | 0.68 |
| blind-peer-review | history-issue | grep | 4 | 0.00 | 0.75 | 0.14 |
| blind-peer-review | history-issue | popular | 4 | 0.50 | 0.50 | 0.50 |
| blind-peer-review | all | codecite | 36 | 0.33 | 0.89 | 0.46 |
| blind-peer-review | all | grep | 36 | 0.11 | 0.61 | 0.26 |
| blind-peer-review | all | popular | 36 | 0.25 | 0.44 | 0.30 |
| codecite | history-title | codecite | 28 | 0.18 | 0.64 | 0.29 |
| codecite | history-title | grep | 28 | 0.07 | 0.36 | 0.14 |
| codecite | history-title | popular | 28 | 0.32 | 0.82 | 0.48 |
| codecite | history-issue | codecite | 16 | 0.19 | 0.44 | 0.24 |
| codecite | history-issue | grep | 16 | 0.06 | 0.25 | 0.09 |
| codecite | history-issue | popular | 16 | 0.19 | 0.94 | 0.43 |
| codecite | all | codecite | 44 | 0.18 | 0.57 | 0.27 |
| codecite | all | grep | 44 | 0.07 | 0.32 | 0.12 |
| codecite | all | popular | 44 | 0.27 | 0.86 | 0.47 |
| identity-model | history-title | codecite | 37 | 0.19 | 0.78 | 0.38 |
| identity-model | history-title | grep | 37 | 0.08 | 0.57 | 0.23 |
| identity-model | history-title | popular | 37 | 0.19 | 0.57 | 0.26 |
| identity-model | history-issue | codecite | 11 | 0.18 | 0.55 | 0.29 |
| identity-model | history-issue | grep | 11 | 0.18 | 0.45 | 0.21 |
| identity-model | history-issue | popular | 11 | 0.55 | 0.64 | 0.56 |
| identity-model | all | codecite | 48 | 0.19 | 0.73 | 0.36 |
| identity-model | all | grep | 48 | 0.10 | 0.54 | 0.22 |
| identity-model | all | popular | 48 | 0.27 | 0.58 | 0.33 |
| identity-stack | history-title | codecite | 34 | 0.21 | 0.62 | 0.35 |
| identity-stack | history-title | grep | 34 | 0.03 | 0.44 | 0.14 |
| identity-stack | history-title | popular | 34 | 0.00 | 0.47 | 0.09 |
| identity-stack | history-issue | codecite | 12 | 0.25 | 0.58 | 0.36 |
| identity-stack | history-issue | grep | 12 | 0.08 | 0.33 | 0.13 |
| identity-stack | history-issue | popular | 12 | 0.00 | 0.42 | 0.08 |
| identity-stack | all | codecite | 46 | 0.22 | 0.61 | 0.35 |
| identity-stack | all | grep | 46 | 0.04 | 0.41 | 0.14 |
| identity-stack | all | popular | 46 | 0.00 | 0.46 | 0.09 |
| terraform-provider-descope | history-title | codecite | 31 | 0.35 | 0.84 | 0.53 |
| terraform-provider-descope | history-title | grep | 31 | 0.19 | 0.61 | 0.34 |
| terraform-provider-descope | history-title | popular | 31 | 0.13 | 0.81 | 0.36 |
| terraform-provider-descope | history-issue | codecite | 1 | 1.00 | 1.00 | 1.00 |
| terraform-provider-descope | history-issue | grep | 1 | 0.00 | 1.00 | 0.12 |
| terraform-provider-descope | history-issue | popular | 1 | 0.00 | 0.00 | 0.00 |
| terraform-provider-descope | all | codecite | 32 | 0.38 | 0.84 | 0.55 |
| terraform-provider-descope | all | grep | 32 | 0.19 | 0.62 | 0.34 |
| terraform-provider-descope | all | popular | 32 | 0.12 | 0.78 | 0.35 |

- blind-peer-review: chunks: 1389 written · elapsed: 0.26s
- codecite: chunks: 2300 written · elapsed: 0.48s
- identity-model: chunks: 10001 written · elapsed: 1.45s
- identity-stack: chunks: 3920 written · elapsed: 0.63s
- terraform-provider-descope: chunks: 3835 written · elapsed: 0.58s

## Cost per query

| Repository | Queries | codecite bytes | codecite ms | rg -n bytes | rg -n ms | rg -l bytes | rg -l ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| blind-peer-review | 36 | 743 | 7.5 (10.0) | 224971 | 6.6 (13.0) | 4574 | 5.9 (7.1) |
| codecite | 44 | 724 | 12.7 (28.8) | 519336 | 11.5 (20.4) | 4843 | 6.7 (9.4) |
| identity-model | 48 | 709 | 42.5 (63.1) | 346853 | 10.0 (31.4) | 16160 | 9.4 (14.5) |
| identity-stack | 46 | 654 | 19.8 (35.2) | 107254 | 9.6 (13.9) | 5746 | 8.4 (11.3) |
| terraform-provider-descope | 32 | 675 | 11.7 (18.4) | 26093 | 7.3 (20.7) | 2019 | 6.9 (15.4) |
