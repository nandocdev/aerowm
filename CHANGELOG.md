# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased] - 2026-09-06

### Added
- **aerowm**: Base Wayland compositor entry point utilizing `smithay`, `calloop`, and `tracing`.
- **aerowm-ipc**: Defined IPC JSON-RPC message contracts using `serde`.
- **aerowm-ctl**: Implemented `clap`-based CLI tool to interact with the Unix Socket.
- **aerowm-lua**: Initialized Luau scripting engine using `mlua`.
- **aerowm-lua**: Implemented `ScriptEngine` with API bindings, dynamic configuration loading, and hook emission system.
- **aerowm-core**: Implemented layout algorithms (`MonadTall`, `Columns`, `Max`) with unit tests.
- **Workspace**: Configured Cargo virtual workspace to host modular crates.
- **aerowm-core**: Initialized the core library crate.
- **aerowm-core**: Implemented basic geometry primitives (`Point`, `Size`, `Rect`).
- **aerowm-core**: Defined the base `Layout` trait for purely functional layout algorithms.
- **aerowm**: Migrated the main binary crate into the `crates/` structure.
- **Changelog**: Added `CHANGELOG.md` to track project evolution.
