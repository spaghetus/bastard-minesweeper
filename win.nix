with import <nixpkgs> {
  crossSystem = {
    config = "x86_64-w64-mingw32";
  };
}; let
  deps = [
    windows.pthreads
  ];
in
  mkShell {
    buildInputs = deps;
    LD_LIBRARY_PATH = lib.makeLibraryPath deps;
  }
