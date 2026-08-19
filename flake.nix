{
  description = "rv — a jj-native terminal branch reviewer";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "rv";
          version = "1.0.0";
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.makeWrapper ];
          # difftastic is found on PATH at run time and degraded past gracefully,
          # but a nix-installed rv should not degrade for want of a wrapper.
          postInstall = ''
            wrapProgram $out/bin/rv \
              --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.difftastic ]}
          '';

          # The suite spawns a jj workspace per test and takes minutes; it runs
          # in the dev shell (`cargo test --workspace`), not in every build.
          doCheck = false;

          meta = {
            description = "A jj-native terminal branch reviewer";
            homepage = "https://github.com/Firaenix/rv";
            license = with pkgs.lib.licenses; [ mit asl20 ];
            mainProgram = "rv";
          };
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rust-analyzer
            clippy
            rustfmt
            difftastic
            jujutsu
          ];
        };
      });
}
