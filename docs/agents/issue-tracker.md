# Issue Tracker

This repository tracks work in **GitHub Issues** on [`brokspace/brok`](https://github.com/brokspace/brok/issues). Skills that read or write issues (`/triage`, `/to-spec`, `/to-tickets`, `/implement`) use the GitHub CLI.

## Everyday commands

```sh
gh issue list --repo brokspace/brok --label needs-triage           # the triage queue
gh issue list --repo brokspace/brok --label ready-for-agent        # work an agent can pick up
gh issue view 42 --repo brokspace/brok --comments
gh issue create --repo brokspace/brok --title "..." --body-file spec.md --label enhancement --label needs-triage
gh issue edit 42 --repo brokspace/brok --remove-label needs-triage --add-label ready-for-agent
gh issue close 42 --repo brokspace/brok --reason "not planned" --comment "..."
```

Label strings come from [triage-labels.md](triage-labels.md).

## Specs and tickets

- A **spec** is one issue labelled `enhancement` whose body is the spec.
- **Tickets** produced from a spec are separate issues. Link them to the spec as GitHub sub-issues when possible, otherwise reference the spec with `Part of #<spec>` in the body. Blocking edges are written as `Blocked by #<n>` lines.

## Pull requests

External pull requests are a request surface too: **a PR is an issue with attached code**. They go through the same roles and states as issues.

- **External** means the author is not a member of the [brokspace](https://github.com/brokspace) organization. Discovery (`/triage` showing what needs attention) lists only external PRs; a member's in-flight PR is not triage work.
- An explicitly named PR (`/triage #57`) is triaged regardless of author.
- For a PR, `ready-for-agent` means a brief is attached and an agent should take the next step on the diff; `ready-for-human` means it is ready for a maintainer to merge.

## Cross-repository work

Brokspace is a polyrepo. When an issue really belongs to another repository, transfer it (`gh issue transfer 42 brokspace/<repo>`) instead of duplicating it. Protocol and manifest changes start in [brokspace/brok-sdk](https://github.com/brokspace/brok-sdk/issues).

## Questions

Questions and open-ended discussion go to [GitHub Discussions on brokspace/brok](https://github.com/brokspace/brok/discussions), not issues.
