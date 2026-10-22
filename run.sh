#!/usr/bin/env bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
cd "$DIR"

# Запуск в режиме Release для сверхбыстрого 60-144 FPS воспроизведения видео
if command -v cargo >/dev/null 2>&1; then
    echo "⚡ Запуск Verizon NLE (Wayland Native, Optimized Release)..."
    cargo run --release --bin verizon_nle
else
    echo "⚡ Запуск Verizon NLE через NixOS окружение (Optimized Release)..."
    exec nix-shell "$DIR/shell.nix" --run "cargo run --release --bin verizon_nle"
fi
