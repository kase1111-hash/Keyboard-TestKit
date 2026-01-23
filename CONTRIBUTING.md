# Contributing to Keyboard TestKit

Thank you for your interest in contributing to Keyboard TestKit! This document provides guidelines and instructions for contributing.

## Table of Contents

- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Changes](#making-changes)
- [Code Style](#code-style)
- [Testing](#testing)
- [Submitting Changes](#submitting-changes)
- [Reporting Issues](#reporting-issues)

## Getting Started

1. Fork the repository on GitHub
2. Clone your fork locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/Keyboard-TestKit.git
   cd Keyboard-TestKit
   ```
3. Add the upstream repository as a remote:
   ```bash
   git remote add upstream https://github.com/kase1111-hash/Keyboard-TestKit.git
   ```

## Development Setup

### Prerequisites

- [Rust](https://rustup.rs/) 1.70 or later
- Platform-specific dependencies:

**Linux (Debian/Ubuntu):**
```bash
sudo apt install libx11-dev libxi-dev libxtst-dev
```

**Linux (Fedora):**
```bash
sudo dnf install libX11-devel libXi-devel libXtst-devel
```

**Linux (Arch):**
```bash
sudo pacman -S libx11 libxi libxtst
```

### Building

```bash
# Debug build (faster compilation)
make debug

# Release build (optimized)
make release

# Run tests
make test
```

### Running

```bash
# Debug mode
make run

# Release mode
make run-release
```

## Making Changes

1. Create a new branch for your changes:
   ```bash
   git checkout -b feature/your-feature-name
   # or
   git checkout -b fix/issue-description
   ```

2. Make your changes, following the [code style guidelines](#code-style)

3. Test your changes thoroughly

4. Commit your changes with a clear, descriptive message:
   ```bash
   git commit -m "Add feature: description of the feature"
   # or
   git commit -m "Fix: description of the bug fix"
   ```

## Code Style

This project follows standard Rust conventions:

### Formatting

All code must be formatted with `rustfmt`:

```bash
make fmt
# or
cargo fmt
```

### Linting

All code must pass `clippy` without warnings:

```bash
cargo clippy --all-targets -- -D warnings
```

### Guidelines

- Use meaningful variable and function names
- Write documentation comments for public APIs
- Keep functions focused and reasonably sized
- Prefer clarity over cleverness
- Handle errors appropriately using `anyhow` or `thiserror`
- Avoid `unwrap()` in production code; use proper error handling

### Module Organization

- `src/keyboard/` - Keyboard input handling and event processing
- `src/tests/` - Individual test implementations
- `src/ui/` - Terminal UI components and widgets
- `src/config.rs` - Configuration structures
- `src/report.rs` - Report generation and export

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_name
```

### Writing Tests

- Place unit tests in the same file as the code being tested
- Use descriptive test names that explain what is being tested
- Test both success and failure cases
- Mock external dependencies when appropriate

## Submitting Changes

### Pull Request Process

1. Ensure your code passes all tests and lints:
   ```bash
   make test
   ```

2. Update documentation if you've changed APIs or added features

3. Push your branch to your fork:
   ```bash
   git push origin feature/your-feature-name
   ```

4. Open a Pull Request against the `main` branch

5. Fill out the PR template with:
   - Description of changes
   - Related issue numbers (if applicable)
   - Testing performed
   - Screenshots (for UI changes)

### PR Guidelines

- Keep PRs focused on a single feature or fix
- Write a clear title and description
- Reference any related issues
- Be responsive to review feedback
- Squash commits if requested

## Reporting Issues

### Bug Reports

When reporting a bug, please include:

- Operating system and version
- Rust version (`rustc --version`)
- Steps to reproduce the issue
- Expected behavior
- Actual behavior
- Any error messages or logs

### Feature Requests

When requesting a feature:

- Describe the use case
- Explain how it would benefit users
- Consider how it fits with existing functionality
- Note any potential implementation challenges

## Questions?

If you have questions about contributing, feel free to:

- Open a discussion on GitHub
- Comment on a related issue
- Reach out to the maintainers

Thank you for contributing to Keyboard TestKit!
