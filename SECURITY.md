# Security Policy

## Supported Versions

Security fixes are prioritized for the most recent release line.

| Version | Supported |
|---|---|
| 0.1.x | yes |
| < 0.1.0 | no |

## Reporting a Vulnerability

Do not open public issues for suspected vulnerabilities.

Report privately to the maintainers with:

- Affected version and environment
- Reproduction steps or proof-of-concept
- Impact assessment (confidentiality/integrity/availability)
- Proposed remediation (if available)

Acknowledgement target: within 3 business days.
Initial triage target: within 7 business days.

## Security Expectations

When changing runtime behavior, preserve:

- Workspace path isolation and shared-workspace grant enforcement
- Queue durability and crash-safe lifecycle moves
- Secret handling with no secret material logged
- Clear operator-facing remediation for unsupported or unsafe commands
