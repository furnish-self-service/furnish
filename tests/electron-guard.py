#!/usr/bin/env python3
"""Run a Linux shared-runtime guard smoke test: python3 tests/electron-guard.py FURNISH ELECTRON."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


@contextmanager
def launch(runtime, package, other_argument, *flags):
    # Electron also accepts an unpacked directory at the distribution's app.asar path.
    package.mkdir(parents=True)
    (package / "package.json").write_text('{"name":"guard-fixture","version":"1.0.0","main":"main.js"}')
    (package / "main.js").write_text("""
        require('electron').app.whenReady().then(() => {
            require('node:fs').writeFileSync(process.env.FURNISH_FIXTURE_READY, 'ready');
            setInterval(() => {}, 60000);
        });
    """)
    ready = package.parent / "ready"
    log = package.parent / "electron.log"
    with log.open("w") as output:
        process = subprocess.Popen(
            [runtime, "--ozone-platform=headless", "--no-sandbox", "--disable-gpu",
             f"--user-data-dir={package.parent / 'user-data'}", *flags, str(package), str(other_argument)],
            env={**os.environ, "FURNISH_FIXTURE_READY": str(ready), "XDG_CONFIG_HOME": str(package.parent)},
            stdout=output, stderr=subprocess.STDOUT,
        )
        try:
            deadline = time.monotonic() + 15
            while not ready.exists():
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError(f"Electron failed to start the fixture: {log.read_text()}")
                time.sleep(0.1)
            yield
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def check_guard(furnish, runtime, *flags):
    with tempfile.TemporaryDirectory(prefix="furnish-electron-") as temp:
        root = Path(temp)
        name = f"Notes-{root.name}"
        target = root / "opt" / name / "resources/app.asar"
        other = root / "lib" / "OtherApp" / "app.asar"
        settings = root / "settings.json"
        settings.write_text("{}")
        props = {"path": str(settings), "location": "$.enabled", "value": True, "requiresClosed": [name.lower()]}

        def apply():
            return subprocess.run(
                [furnish, "dsc-resource", "value", "set"], input=json.dumps(props), text=True,
                capture_output=True, timeout=15,
                env={**os.environ, "HOME": temp, "FURNISH_POLICY_FILE": str(root / "policy.json"),
                     "FURNISH_SYSTEM_DIR": str(root / "system"), "FURNISH_DATA_DIR": str(root / "data")},
            )

        with launch(runtime, other, target):
            with launch(runtime, target, other, *flags):
                blocked = apply()
                assert blocked.returncode and name.lower() in blocked.stderr, blocked
                assert settings.read_text() == "{}"
            applied = apply()
            assert applied.returncode == 0, applied.stderr
            assert json.loads(settings.read_text()) == {"enabled": True}


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    check_guard(*sys.argv[1:])
    check_guard(*sys.argv[1:], "--autostart")
    print("Passed: the guarded app blocked writes on normal launch and autostart; the unrelated app allowed them.")
