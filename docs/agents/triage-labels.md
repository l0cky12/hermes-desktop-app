# Triage Labels

The skills speak in terms of five triage roles. This repo uses the same names as labels:

| Label             | Meaning                                  |
| ----------------- | ---------------------------------------- |
| `needs-triage`    | Maintainer needs to evaluate this issue  |
| `needs-info`      | Waiting on reporter for more information |
| `ready-for-agent` | Fully specified, ready for an AFK agent  |
| `ready-for-human` | Requires human implementation            |
| `wontfix`         | Will not be actioned                     |

`/wayfinder` also uses `wayfinder:map`, `wayfinder:research`, `wayfinder:prototype`, `wayfinder:grilling`, and `wayfinder:task`.

If a label is missing from the tracker (a fresh fork, say), create them all:

```sh
for l in needs-triage needs-info ready-for-agent ready-for-human wayfinder:map wayfinder:research wayfinder:prototype wayfinder:grilling wayfinder:task; do gh label create "$l" --force; done
```
