# EmelkaNotch (Rust + egui)

A runnable starter for a macOS-notch-inspired Windows overlay. It creates a small, borderless, transparent, always-on-top window near the top center of the primary display.

## Included in this starter

- Click the pill to expand/collapse the panel.
- Music tab with demo play/pause state (not connected to a real media session).
- System tab with live CPU and RAM readings via `sysinfo`.
- Notifications tab as an integration placeholder.
- Build tab as a placeholder/demo status (does not run a real build).
- Windows primary-screen positioning.

## Requirements

- Windows 10/11
- Rust toolchain installed with `rustup`
- Visual Studio Build Tools with the **Desktop development with C++** workload (MSVC linker)

Install Rust from <https://rustup.rs/>. Verify:

```powershell
rustc --version
cargo --version
```

## Run in development

Open PowerShell in this folder:

```powershell
cargo run
```

First build downloads dependencies, so it may take a few minutes.

## Build a release executable

```powershell
cargo build --release
```

Executable:

```text
target\release\EmelkaNotch.exe
```

## Notes / next steps

This is a starter, not a complete Dynamic Island replacement. The music controls are local demo state; notifications are not yet subscribed to Windows; the build panel does not yet launch a real build. Those integrations should be implemented as separate modules using Windows APIs and background workers so the UI remains responsive.

The initial positioning uses the primary display's width. Multi-monitor awareness, DPI scaling, click-through behavior outside the pill, startup-on-login, and installer packaging are future improvements.
