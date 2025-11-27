{ pkgs ? import <nixpkgs> {} }:

let
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

    # LLM runtime
    ollama
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
