{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    rustc
    cargo
    rustfmt
    clippy

    cloc
    scc
  ];

  shellHook = ''
    echo "Entering Nix shell..."
    echo "Rust version: $(rustc --version)"
    echo "Cargo version: $(cargo --version)"
  '';
}
