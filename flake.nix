{
  description = "Launchpad process manager workspace";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "launchpad";
            version = "0.1.0";

            # Point to your workspace root directory
            src = ./.;

            # You need to provide the cargo lock hash.
            # If you haven't generated a Cargo.lock yet, run 'cargo generate-lockfile' locally first.
            cargoLock = {
              lockFile = ./Cargo.lock;
            };

            # Optional: if your build requires system dependencies (like OpenSSL, pkg-config, etc.),
            # list them here under nativeBuildInputs or buildInputs.
            # nativeBuildInputs = with pkgs; [ pkg-config ];
            # buildInputs = with pkgs; [ openssl ];
          };
        }
      );

      # Optional: Provides a development shell with cargo, rustc, etc. ready to go
      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            buildInputs = with pkgs; [
              rustc
              cargo
              rustfmt
              clippy
            ];
          };
        }
      );
    };
}
