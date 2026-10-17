{ buildGoModule, lib }:

buildGoModule {
  pname = "orby-runner";
  version = "0.1.0";
  src = ./.;
  vendorHash = null;

  meta = {
    description = "OrbStack machine lifecycle and SSH proxy for Orby";
    license = lib.licenses.mit;
    platforms = lib.platforms.darwin;
    mainProgram = "orby-runner";
  };
}
