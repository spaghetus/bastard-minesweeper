{pkgs ? import <nixpkgs> {}}:
with pkgs; let
  libPath = lib.makeLibraryPath [
    libGL
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
  ];
in
  mkShell {
    buildInputs = [
      xorg.libxcb
    ];
    LD_LIBRARY_PATH = libPath;
  }
