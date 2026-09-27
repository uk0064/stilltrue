# Security policy

## Supported versions

No release has been published yet, so fixes land on `main`. Once releases exist, only
the latest one receives security fixes.

## Reporting a vulnerability

Please do not open a public issue. Report it privately instead, through
[GitHub's private vulnerability reporting](https://github.com/uk0064/stilltrue/security/advisories/new)
— the "Report a vulnerability" button on this repository's Security tab.

Say what you ran, against what input, what happened and what you expected. The project
is maintained by one person: a report is acknowledged once it has been read, and the
advisory is where the fix, or the reason there will not be one, is recorded. Credit is
given in the advisory unless you ask otherwise.

## What is in scope

stilltrue reads repositories it did not write, and some of its output reaches coding
agents. Of most interest:

- a document or repository that makes stilltrue run a program, or read or write a file,
  beyond what its documentation describes — including through the arguments it passes
  to git, and through `--fix`;
- anything that lets a crafted repository put text of its choosing into an agent's
  session through `stilltrue-hook`, beyond the findings themselves;
- the GitHub Action, and the release archives and installers.

Findings the tool reports wrongly, or fails to report, are ordinary bugs: open an issue.
