let
  # Use stable nixpkgs for most packages
  pkgs = import <nixpkgs> {};

  # Use unstable for Ollama only (to get latest version)
  pkgs-unstable = import (builtins.fetchTarball {
    url = "https://github.com/NixOS/nixpkgs/archive/nixpkgs-unstable.tar.gz";
  }) {};

  # Use latest stable Rust
  rust-overlay = import (builtins.fetchTarball "https://github.com/oxalica/rust-overlay/archive/master.tar.gz");
  pkgs' = import <nixpkgs> { overlays = [ rust-overlay ]; };
  rust = pkgs'.rust-bin.stable.latest.default;
in

pkgs'.mkShell {
  buildInputs = with pkgs'; [
    # Rust toolchain (latest stable)
    rust
    rust-analyzer

    # Build tools
    pkg-config
    openssl
  ] ++ [
    # LLM runtime (from unstable for latest version)
    pkgs-unstable.ollama
  ];

  shellHook = ''
    echo "llm-tui development environment"
    echo "Rust version: $(rustc --version)"
    echo ""
    echo "To start Ollama server: ollama serve"
    echo "To pull a model: ollama pull llama2"
  '';

  PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
}
