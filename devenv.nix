{
  pkgs,
  ...
}:

{
  # https://devenv.sh/basics/
  env.GREET = "devenv";

  overlays = [
    (final: prev: {
      esp-nvs-partition-tool = final.callPackage ./.nix/esp-nvs-partition-tool.nix { };
    })
  ];

  # https://devenv.sh/packages/
  packages = with pkgs; [
    actionlint
    cargo-edit
    esp-nvs-partition-tool
    git
    just
    nixfmt
  ];

  languages.rust = {
    enable = true;
    channel = "stable";
    version = "1.93.1";
    components = [
      "rustc"
      "rust-src"
      "cargo"
      "clippy"
      "rustfmt"
      "rust-analyzer"
    ];
    targets = [
      "x86_64-unknown-linux-gnu"
      "riscv32imac-unknown-none-elf"
      "riscv32imc-unknown-none-elf"
    ];
  };
}
