# Security policy

## Reporting a vulnerability

**Please do not open a public issue for a security report.**

Use GitHub's private reporting: go to the repository's **Security** tab and
choose **Report a vulnerability**. That opens a private advisory visible only
to the maintainers.

If that option is not available to you, open a normal issue that says only
that you have a security report and asks for a private channel — no details,
no reproduction steps, no affected versions. A maintainer will reply with
somewhere private to send them.

Expect an acknowledgement within a week. There is no bug-bounty programme.

## What is in scope

CodeLore is a local command-line tool. It reads a git repository, writes a
fact store into the user's cache directory, and emits reports. It has no
server, no account, and makes no network calls unless you opt into the
advisory narrative layer.

In scope:

- Anything that lets a **repository being analysed** affect the machine
  running the analysis beyond the documented outputs — code execution, writes
  outside the cache and the paths you passed, or reads of files the analysis
  has no reason to touch.
- Repository-controlled content reaching an output in a form that executes or
  misleads: script injection into the single-page dashboard, escaping in the
  Markdown and SARIF emitters, or content that forges the grounding stamp on
  an advisory narrative.
- Secrets leaking from the process — into a report, the fact store, the
  provenance sidecar, or a log line.
- Supply-chain integrity of what we publish: the release archives, their
  attestations, the container image, and the crates on crates.io.

Out of scope:

- Denial of service caused by the size of the repository you point it at. A
  large history costs time and memory by design; the documented caps are cost
  controls, not a security boundary.
- Analysis results you disagree with. Every formula is published — `codelore
  explain <topic>` prints it — and a wrong number is a correctness bug, not a
  vulnerability.
- Findings that require an attacker to already control the machine, the cache
  directory, or the CodeLore binary itself.

## Analysing repositories you do not trust

Whether analysing a hostile repository is a supported use is still an open
question for this project, and until it is answered, treat it as **not**
supported. A repository controls its own file contents, paths, author names,
commit messages, and the configuration files CodeLore reads from its root
(`.mailmap`, `.codelorebots`, `.codelore-teams`, `.codeloreignore`,
`.codelore-thresholds.toml`). All of that reaches the parsers and the
reports. If you analyse untrusted code, do it in a sandbox you would be
willing to lose.

Reports about this class are welcome and will be treated as scoping input
rather than dismissed.

## Supported versions

Fixes land on `main` and ship in the next release. This is a pre-1.0 project:
patches are not backported to earlier versions.
