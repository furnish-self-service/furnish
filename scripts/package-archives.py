"""Build the release archives from the same payloads as the platform installers."""
import argparse
from pathlib import Path
import platform
import plistlib
import tarfile
from zipfile import ZipFile, ZIP_DEFLATED


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("system", choices=["linux", "macos", "windows"])
    parser.add_argument("payload", type=Path, help="extracted Linux usr/, macOS .app, or Windows install directory")
    parser.add_argument("version")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    root = args.payload
    arch = platform.machine().lower()
    arch = {"amd64": "x86_64", "aarch64": "arm64"}.get(arch, arch)
    prefix = f"furnish_{args.version}_{args.system}-{arch}"
    args.output.mkdir(parents=True, exist_ok=True)
    bin_dir = {"linux": root / "bin", "macos": root / "Contents/MacOS", "windows": root}[args.system]
    ext = ".exe" if args.system == "windows" else ""
    gui = "furnish-gui" + ext
    if args.system == "macos":
        with (root / "Contents/Info.plist").open("rb") as stream:
            gui = plistlib.load(stream)["CFBundleExecutable"]
        if gui == "furnish" or Path(gui).name != gui:
            parser.error(f"invalid macOS desktop executable: {gui}")
    repo = Path(__file__).resolve().parents[1]
    files = {
        "furnish" + ext: bin_dir / ("furnish" + ext),
        "furnish-gui" + ext: bin_dir / gui,
        "LICENSE": repo / "LICENSE",
        "THIRD-PARTY-LICENSES.html": repo / "THIRD-PARTY-LICENSES.html",
    }
    required = list(files.values())
    if args.system == "linux":
        required.extend(root / path for path in (
            "lib/furnish/dsc/dsc", "lib/furnish/dsc/filecontent", "lib/furnish/THIRD-PARTY-LICENSES.html",
            "share/applications/furnish.desktop", "share/icons/hicolor/128x128/apps/furnish-gui.png",
        ))
    for path in required:
        if not path.is_file() or not path.stat().st_size:
            parser.error(f"missing or empty release payload: {path}")
    if args.system == "windows":
        with ZipFile(args.output / f"{prefix}_binaries.zip", "w", compression=ZIP_DEFLATED) as out:
            for name, file in files.items():
                out.write(file, name)
    else:
        with tarfile.open(args.output / f"{prefix}_binaries.tar.gz", "w:gz") as out:
            for name, file in files.items():
                out.add(file, arcname=name)
    if args.system == "linux":
        with tarfile.open(args.output / f"{prefix}_user.tar.gz", "w:gz") as out:
            for name in ("bin", "lib/furnish", "share/applications", "share/icons"):
                out.add(root / name, arcname="furnish/" + name)
            out.add(repo / "LICENSE", arcname="furnish/lib/furnish/LICENSE")


if __name__ == "__main__":
    main()
