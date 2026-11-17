{ outputs = _: { furnish = builtins.fromJSON (builtins.readFile ./apps.json); }; }
