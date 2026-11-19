# Alice's offline flake opens the exported Furnish store path with path:, which
# carries no Git metadata. Prebuild her generations with that same source and
# the requested apps, including when the test itself comes from a Git checkout.
furnish: apps:
let
  inputs = furnish.inputs // { inherit apps; };
  sourceInfo = { inherit (furnish) outPath; };
  self = sourceInfo // outputs // { inherit inputs sourceInfo; };
  outputs = (import "${furnish.outPath}/flake.nix").outputs (inputs // { inherit self; });
in outputs.nixosModules.default
