# Triage Labels

The Brokspace skills (`/triage`, `/to-tickets`, `/to-spec`, ...) speak in terms of five canonical triage roles and two category roles. This file maps each role to the label string used in this repository's issue tracker.

## State roles

An issue carries exactly one state role at a time.

| Canonical role    | Label in brokspace/brok | Meaning                                              |
| ----------------- | ----------------------- | ---------------------------------------------------- |
| `needs-triage`    | `needs-triage`          | A maintainer needs to evaluate this issue            |
| `needs-info`      | `needs-info`            | Waiting on the reporter for more information         |
| `ready-for-agent` | `ready-for-agent`       | Fully specified, ready for an AFK coding agent       |
| `ready-for-human` | `ready-for-human`       | Needs a human: judgment, access or manual testing    |
| `wontfix`         | `wontfix`               | Will not be actioned (closed as "not planned")       |

## Category roles

An issue carries exactly one category role.

| Canonical role | Label in brokspace/brok | Meaning                          |
| -------------- | ----------------------- | -------------------------------- |
| `bug`          | `bug`                   | Something is broken              |
| `enhancement`  | `enhancement`           | New capability or improvement    |

## Supporting labels

These never replace a role; they help people find work.

| Label              | Meaning                                         |
| ------------------ | ----------------------------------------------- |
| `good first issue` | Small, well scoped, a good entry point          |
| `help wanted`      | Maintainers would welcome a contribution        |
| `documentation`    | Docs only                                       |
| `dependencies`     | Dependency updates (Dependabot)                 |
| `area:*`           | Component: `cli`, `index`, `bus`, `shards`, `skills`, `integrations` |

## State machine

```text
(new issue) ──► needs-triage ──► ready-for-agent
                    ▲   │   └──► ready-for-human
                    │   ├──────► wontfix (closed, not planned)
                    │   ▼
                    └─ needs-info ──(14 days, no reply)──► closed, not planned
```

- Every new issue gets `needs-triage` automatically (issue forms and `.github/workflows/triage-label.yml`).
- A reporter comment on a `needs-info` issue moves it back to `needs-triage` (`.github/workflows/needs-info.yml`).
- Judge every issue against [SCOPE.md](https://github.com/brokspace/brok/blob/main/SCOPE.md) before choosing a state. Rejected enhancements get a file in [`.out-of-scope/`](https://github.com/brokspace/brok/tree/main/.out-of-scope) and close as "not planned".

When a skill mentions a role (for example "apply the AFK-ready triage label"), use the label string from these tables.
