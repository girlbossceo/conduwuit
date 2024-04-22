{ inputs

# Dependencies
, dockerTools
, lib
, main
, stdenv
, tini
}:

dockerTools.buildLayeredImage {
  name = main.pname;
  tag = "main";
  created = "@${toString inputs.self.lastModified}";
  contents = [
    dockerTools.caCertificates
  ];
  config = {
    Entrypoint = if !stdenv.isDarwin
      # Use the `tini` init system so that signals (e.g. ctrl+c/SIGINT)
      # are handled as expected
      then [ "${lib.getExe' tini "tini"}" "--" ]
      else [];
    Cmd = [
      "${lib.getExe main}"
    ];
  };
}
