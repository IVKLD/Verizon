# Strict Wayland-Only Policy (NO X11 / NO XWayland)

## Core Mandate
- **STRICTLY PURE WAYLAND ONLY**: NEVER write code for X11, XWayland, or X11 fallbacks.
- **NO X11 Environment Overrides**: NEVER set `WINIT_UNIX_BACKEND="x11"` or force X11/XWayland in any script, command, or source code. Always use native Wayland (`wayland`).
- **NO X11 Crate Features**: In `Cargo.toml`, never enable `"x11"` features on GUI or windowing libraries (`eframe`, `winit`, `egui`, etc.). Enable only `"wayland"`.
- **NO X11 Dependencies in Nix**: Remove all direct X11 packages (`libx11`, `libXcursor`, `libXi`, `libXrandr`) from `shell.nix`. Target pure Wayland packages (`wayland`, `wayland-protocols`, `libxkbcommon`).
- **Native Wayland Architecture**: All windowing, surface presentation, input, drag-and-drop, and clipboards must be designed strictly for pure modern Wayland compositors.

## Git Workflow
- **Git Push on Demand**: Whenever requested to push/upload sources, always stage, commit, and push to GitHub repository `git@github.com:IVKLD/Verizon.git` on branch `main`.
