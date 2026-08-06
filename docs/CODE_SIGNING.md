# Code signing policy — meetily++

## Project lineage

meetily++ is a downstream fork of [Meetily](https://github.com/Zackriya-Solutions/meeting-minutes)
(MIT), tracking upstream v0.4.0 and adding audited community-fork work plus original
features. Every upstream and third-party fork change merged into this tree was reviewed
line by line before inclusion (five independent security audits, all recorded in the
repository history and in `HANDOFF.md`).

## Team and roles

- **Maintainer, release manager, and signing authority:** Anthony Clendenen
  (GitHub [@anthonyonazure](https://github.com/anthonyonazure))

The maintainer owns this repository, authors and reviews all changes merged into it,
controls the release pipeline, and is the only person authorized to trigger signed
builds. There are no other team members with write or release access.

## How builds are produced

- Builds run only in GitHub Actions from this repository, on workflow_dispatch by the
  maintainer, from committed source.
- Every third-party GitHub Action is pinned to a full commit SHA; a CI gate fails the
  build if any action reference is not SHA-pinned.
- Rust dependencies are policed by `cargo-deny` (unknown registries and unknown git
  sources are denied); native build inputs are hash-pinned where the upstream toolchain
  allows it.
- The build is reproducible from the tagged commit: artifacts published on a release
  correspond to the workflow run recorded on that tag.

## Release integrity today

Releases are published at
https://github.com/anthonyonazure/meetily-merged/releases and each updater artifact
carries a minisign signature produced by the maintainer's own key. The desktop app's
auto-updater trusts only that key and only this repository's release feed.

## Privacy

meetily++ processes meeting audio, transcripts, and summaries entirely on the user's
device or through an endpoint the user configures. It ships no telemetry and no
analytics. Network egress happens only for: model downloads from official upstream
sources, LLM requests to the user's configured provider, Microsoft Graph calls after the
user explicitly signs in, and explicit per-meeting share actions the user triggers.
See [PRIVACY_POLICY.md](../PRIVACY_POLICY.md).

## Attribution

If and when this project's Windows binaries are signed through the SignPath Foundation:

> Free code signing provided by [SignPath.io](https://signpath.io), certificate by
> [SignPath Foundation](https://signpath.org).
