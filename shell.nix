{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    cargo
    rustc
    pkg-config
    llvmPackages.libclang
    glibc.dev
    wayland
    wayland-protocols
    libxkbcommon
    vulkan-loader
    vulkan-headers
    libGL
    fontconfig
    pipewire
    alsa-lib
    ffmpeg
  ];

  LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
  BINDGEN_EXTRA_CLANG_ARGS = "-isystem ${pkgs.glibc.dev}/include";

  LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
    llvmPackages.libclang.lib
    wayland
    libxkbcommon
    vulkan-loader
    libGL
    fontconfig
    pipewire
    alsa-lib
    ffmpeg
  ]);
}
