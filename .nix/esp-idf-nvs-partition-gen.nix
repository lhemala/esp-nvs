{ pkgs ? import <nixpkgs> {} }:

let
  lib = pkgs.lib;
  python = pkgs.python313;
  pythonPackages = pkgs.python313Packages;
in

pythonPackages.buildPythonPackage rec {
  pname = "esp-idf-nvs-partition-gen";
  version = "0.2.0";
  pyproject = true;

  src = pkgs.fetchPypi {
    pname = "esp_idf_nvs_partition_gen";
    version = version;
    sha256 = "sha256-0fI86YdsBGnlB7NJkAEmbQ0XU4FYFRomEsxwuAcG3OA=";
  };

  build-system = [
      pythonPackages.setuptools
  ];

  dependencies = [
      pythonPackages.cryptography
      pythonPackages.pyopenssl
  ];

  checkPhase = ''
    ${python.interpreter} -c "import esp_idf_nvs_partition_gen as m; print('loaded', getattr(m,'__version__','no-version'))"
  '';

  meta = with lib; {
    description = "Tool to generate ESP-IDF NVS partition images (nvs_partition_gen.py)";
    homepage = "https://pypi.org/project/esp-idf-nvs-partition-gen";
    license = licenses.mit;
    maintainers = with pkgs.lib.maintainers; [];
    mainProgram = "nvs_partition_gen.py";
  };
}
