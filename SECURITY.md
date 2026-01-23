# Security Policy

## Supported Versions

The following versions of Keyboard TestKit are currently supported with security updates:

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Security Considerations

Keyboard TestKit is a diagnostic utility that interacts with keyboard input at a low level. We take security seriously and have implemented the following safeguards:

### Data Privacy

- **No network transmission**: Keyboard TestKit does not send any keystroke data over the network
- **Local storage only**: All logs and reports are stored locally on your machine
- **No persistent logging**: By default, keystroke data is not persisted after the session ends
- **Export control**: Reports are only generated when explicitly requested by the user

### Permissions

- The application requires access to keyboard input devices
- On Linux, this may require membership in the `input` group or running with elevated privileges
- The application does not require or request internet access

### What We Don't Do

- We do not log passwords or sensitive input in persistent storage
- We do not transmit any data externally
- We do not install background services or daemons
- We do not modify system keyboard settings

## Reporting a Vulnerability

If you discover a security vulnerability in Keyboard TestKit, please report it responsibly:

### How to Report

1. **Do not** open a public issue for security vulnerabilities
2. Email the maintainers directly or use GitHub's private vulnerability reporting feature
3. Include the following information:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

### What to Expect

- **Acknowledgment**: We will acknowledge receipt of your report within 48 hours
- **Assessment**: We will assess the vulnerability and determine its severity
- **Updates**: We will keep you informed of our progress
- **Resolution**: We aim to resolve critical vulnerabilities within 7 days
- **Credit**: We will credit you in the release notes (unless you prefer to remain anonymous)

### Scope

The following are considered in scope:

- Security issues in the Keyboard TestKit codebase
- Vulnerabilities in dependencies that affect Keyboard TestKit
- Privacy concerns related to keystroke handling

The following are out of scope:

- Issues in third-party tools or operating systems
- Social engineering attacks
- Physical security concerns

## Security Best Practices for Users

1. **Download from official sources**: Only download Keyboard TestKit from the official GitHub repository
2. **Verify integrity**: Check file hashes when available
3. **Run with minimal privileges**: Avoid running as root/administrator unless necessary
4. **Review exports**: Be mindful of what data you share from exported reports

## Updates

This security policy may be updated from time to time. Please check back periodically for any changes.

---

Thank you for helping keep Keyboard TestKit and its users safe!
