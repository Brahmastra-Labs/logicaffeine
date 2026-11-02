# Security Policy

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.11.x  | Yes       |
| < 0.11  | No        |

Only the 0.11 release line receives security fixes. Every published crate shares one version (see
[Versioning](../docs/project/VERSIONING.md)), so the table covers all of them.

## Reporting a Vulnerability

If you discover a security vulnerability in Logicaffeine, please report it responsibly. That
includes anything a remote peer can trigger, such as a wire-decoder crash or an allocation bomb:
report it here rather than as a public [challenge](../docs/project/CHALLENGE.md) issue.

**Email:** team@datacenter.dev

**Please do not:**
- Open public GitHub issues for security vulnerabilities
- Disclose the vulnerability publicly before it has been addressed

**We will:**
- Acknowledge receipt within 48 hours
- Provide an estimated timeline for a fix
- Credit you in the release notes (unless you prefer to remain anonymous)

## Scope

This security policy applies to:
- every crate this workspace publishes to crates.io (`logicaffeine-base` through `logicaffeine-cli`)
- the `largo` and `logicaffeine-lsp` binaries, the install scripts, and the VS Code extension
- the web application at logicaffeine.com, including the Studio and the browser runtime it ships
- the package registry (`services/registry`), and the
  pricing-request worker (`services/leads`), which stores the contact details people send from the pricing page

## Out of Scope

- Issues in dependencies (please report to the respective projects)
- Theoretical attacks without proof of concept
