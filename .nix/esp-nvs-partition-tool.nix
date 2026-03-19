{
  lib,
  rustPlatform,
  pkg-config,
}:

rustPlatform.buildRustPackage {
  pname = "esp-nvs-partition-tool";
  version = "0.1.0";

  src = ./..;

  cargoLock = {
    lockFile = ../Cargo.lock;
  };

  # Only build the esp-nvs-partition-tool binary
  cargoBuildFlags = [
    "--bin"
    "esp-nvs-partition-tool"
  ];

  nativeBuildInputs = [ pkg-config ];

  meta = with lib; {
    description = "ESP-IDF compatible NVS partition table parser and generator";
    homepage = "https://github.com/lhemala/esp-nvs/";
    license = with licenses; [
      mit
      asl20
    ];
    maintainers = [ ];
    mainProgram = "esp-nvs-partition-tool";
  };
}
