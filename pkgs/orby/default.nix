{ rustPlatform, lib }:

rustPlatform.buildRustPackage {
  pname = "orby";
  version = "0.1.0";
  src = ./.;
  cargoLock.lockFile = ./Cargo.lock;
  # OrbStack is not accessible inside a Nix build sandbox. `cargo test` on a
  # configured Mac still runs the platform-specific integration tests.
  cargoTestFlags = [ "--lib" ];

  meta = {
    description = "OrbStack machine lifecycle and SSH proxy for Orby";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin;
    mainProgram = "orby";
  };
}
