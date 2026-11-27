# For nixos-switch.nix: switching the VM to alice's system for real would replace the VM's own
# configuration. This switch-to-configuration does to furnish.service what NixOS's does to a
# service whose definition changed: it stops the service, activates, and starts it again, unless
# the unit says X-RestartIfChanged=false and /run/furnish-test/stop doesn't exist (an older
# furnish.service, which didn't say so). It reloads furnish-restart.service, which changes with
# furnish.service, and writes its output as the real one does, to stdout. It leaves
# /run/furnish-test/switched when it got to the end.
{ ... }:
{
  system.switch.enable = false;
  system.systemBuilderCommands = ''
    mkdir -p $out/bin
    cat > $out/bin/switch-to-configuration <<'SWITCH'
    #!/bin/sh
    set -e
    PATH=/run/current-system/sw/bin
    restart=yes
    if grep -qx X-RestartIfChanged=false /etc/systemd/system/furnish.service && ! test -e /run/furnish-test/stop; then
      restart=
    fi
    echo "stopping the following units: furnish.service"
    if [ -n "$restart" ]; then systemctl stop furnish.service; fi
    echo "activating the configuration..."
    if [ -n "$restart" ]; then systemctl start furnish.service; fi
    echo "reloading the following units: furnish-restart.service"
    if [ -e /etc/systemd/system/furnish-restart.service ]; then systemctl reload furnish-restart.service; fi
    echo "$1" > /run/furnish-test/switched
    SWITCH
    chmod +x $out/bin/switch-to-configuration
  '';
}
