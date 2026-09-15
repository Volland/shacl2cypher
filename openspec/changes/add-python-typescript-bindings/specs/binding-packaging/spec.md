## Purpose

Defines how the Python and TypeScript bindings are built, versioned and published, so users install a working package with both database backends and no Rust toolchain.

## ADDED Requirements

### Requirement: Python wheels
The `shacl2cypher` Python distribution SHALL publish binary wheels for CPython 3.9 and newer, using the stable ABI (one wheel per platform). Wheels SHALL cover Linux x86_64 and Linux arm64 on glibc 2.28 or newer, and macOS arm64. Each wheel SHALL include the Neo4j and LadybugDB backends.

#### Scenario: Install without Rust
- **WHEN** `pip install shacl2cypher` runs on Linux x86_64 with CPython 3.12 and no Rust or cmake installed
- **THEN** a wheel installs, and `shacl2cypher.available_backends()` returns `["neo4j", "ladybug"]`

#### Scenario: Oldest supported glibc
- **WHEN** the Linux wheel is installed in a `manylinux_2_28` container
- **THEN** `import shacl2cypher` succeeds and a LadybugDB fixture validates

### Requirement: Python source distribution
The Python release SHALL also publish an sdist that builds with a Rust toolchain. By default it builds with both backends, which requires cmake and a C++ compiler. The build SHALL allow backends to be disabled for a compile-only build.

#### Scenario: Compile-only source build
- **WHEN** the sdist is built with backends disabled on a machine without cmake
- **THEN** the build succeeds and compile works, while opening a database raises `BackendUnavailableError`

### Requirement: npm packages
The `shacl2cypher` npm package SHALL work on Node.js 18 and newer. It SHALL load a prebuilt native addon from an optional per-platform dependency: `shacl2cypher-linux-x64-gnu`, `shacl2cypher-linux-arm64-gnu` or `shacl2cypher-darwin-arm64`. Each addon SHALL include both backends. It SHALL ship ES module and CommonJS entry points and TypeScript declarations.

#### Scenario: Install on a supported platform
- **WHEN** `npm install shacl2cypher` runs on macOS arm64 with Node.js 20
- **THEN** only `shacl2cypher-darwin-arm64` is installed as the platform addon, and both `import` and `require` of `shacl2cypher` work

#### Scenario: Unsupported platform
- **WHEN** the package is loaded on Windows x64
- **THEN** loading throws an error naming the platform and listing the supported platforms

### Requirement: Version lockstep
The Python distribution, the npm packages (main and per-platform) and the Rust crates SHALL share one version. Each binding SHALL expose it as `shacl2cypher.__version__` or the `version` export, equal to the `compilerVersion` it writes into manifests.

#### Scenario: Version agreement
- **WHEN** a manifest is compiled with the Python package version `0.2.0`
- **THEN** `manifest["compilerVersion"]` is `0.2.0`, and the npm packages of the same release also report `0.2.0`

### Requirement: Release publishing
Pushing a `v*` tag SHALL build every wheel, the sdist and every npm package. It SHALL publish them to PyPI and npm only after all builds and their smoke tests succeed. A release SHALL fail without publishing when the tag does not match the workspace version.

#### Scenario: One target fails
- **WHEN** the Linux arm64 wheel build fails during a tagged release
- **THEN** nothing is published to PyPI or npm for that tag

#### Scenario: Tag mismatch
- **WHEN** tag `v0.3.0` is pushed while the workspace version is `0.2.0`
- **THEN** the release workflow fails before publishing and names both versions

### Requirement: Bindings in CI
Continuous integration SHALL build both bindings on every push and pull request. It SHALL run their test suites, including compile parity with the CLI and LadybugDB validation. Neo4j validation tests SHALL run when a Neo4j service is configured.

#### Scenario: Binding regression
- **WHEN** a pull request changes rule naming in the core
- **THEN** the Python and Node parity tests run against the CLI output of the same commit
