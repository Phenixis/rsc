# Security policy

## Supported versions

`rsc` has no release yet. Only the latest commit on `main` is supported: fixes land there, and
nothing is backported.

## What counts as a vulnerability

`rsc` keeps OAuth tokens on disk and will read a SoundCloud `client_secret` from its configuration,
and it drives `mpv` through a local socket. Please report anything that lets someone:

- read or leak the access token, the refresh token or the `client_secret` (in files, logs, error
  messages, command lines or environment);
- store those secrets with looser permissions than intended (`tokens.json` should be mode `0600`);
- write or read files outside the folders `rsc` is meant to use;
- run a command or take control of the player through the `mpv` IPC socket, or through data coming
  from the SoundCloud API;
- get around the OAuth checks (PKCE, `state`, single-use codes and refresh tokens).

Bugs that only affect you (a crash, a wrong track) are not vulnerabilities: use a normal
[issue](https://github.com/Phenixis/rsc/issues/new/choose) for them.

## How to report

**Do not open a public issue for a vulnerability.**

1. Preferred: use GitHub's private reporting. Open the
   [Security tab](https://github.com/Phenixis/rsc/security) of the repository and choose
   **Report a vulnerability**.
2. If that is not available, email [max@maximeduhamel.com](mailto:max@maximeduhamel.com) with
   the subject `rsc security`.

Please include what you found, how to reproduce it, the commit you tested, and what an attacker
could do with it. Never send real tokens or secrets: redact them.

## What to expect

`rsc` is maintained by one person in their spare time, so the commitment is best effort:

- an answer within 7 days;
- a fix or a plan, and the credit you want in the fix, once the problem is confirmed;
- public disclosure after the fix is on `main`, or earlier if you and the maintainer agree.

Testing against your own SoundCloud account or against `rsc-mock` is fine. Please do not test
against other people's accounts, and do not run anything that degrades SoundCloud's service.
