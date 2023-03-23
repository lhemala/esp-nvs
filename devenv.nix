{ pkgs, lib, config, inputs, ... }:

{
  # https://devenv.sh/basics/
  env.GREET = "devenv";

  overlays = [
    (final: prev: {
      esp-idf-nvs-partition-gen = final.callPackage ./.nix/esp-idf-nvs-partition-gen.nix {};
    })
  ];

  # https://devenv.sh/packages/
  packages = [
    pkgs.esp-idf-nvs-partition-gen
    pkgs.git
    pkgs.just
   ];

  languages.rust = {
    enable = true;
    channel = "stable";
    version = "1.90.0";
    components = [ "rustc" "rust-src" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
    targets = [ "x86_64-unknown-linux-gnu" ];
  };

  # https://devenv.sh/processes/
  # processes.cargo-watch.exec = "cargo-watch";

  # https://devenv.sh/services/
  # services.postgres.enable = true;

  scripts.nvs_partition_gen.exec = ''
    python3 -m esp_idf_nvs_partition_gen "$@"
  '';

  # https://devenv.sh/scripts/
#  scripts.hello.exec = ''
#    echo hello from $GREET
#  '';
#
#  enterShell = ''
#    hello
#    git --version
#  '';

  # https://devenv.sh/tasks/
  # tasks = {
  #   "myproj:setup".exec = "mytool build";
  #   "devenv:enterShell".after = [ "myproj:setup" ];
  # };

  # https://devenv.sh/tests/
  enterTest = ''
    echo "Running tests"
    git --version | grep --color=auto "${pkgs.git.version}"
  '';

  # https://devenv.sh/git-hooks/
  # git-hooks.hooks.shellcheck.enable = true;

  # See full reference at https://devenv.sh/reference/options/
}
