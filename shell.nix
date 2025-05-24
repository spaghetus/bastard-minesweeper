{pkgs ? import <nixpkgs> {}}:
with pkgs; let
  deps = [
    libGL
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    llvmPackages_12.bintools
  ];
in
  mkShell {
    buildInputs = deps;
    LD_LIBRARY_PATH = lib.makeLibraryPath deps;
  }
