"""Build the release archives from the same payloads as the platform installers."""
import argparse
from pathlib import Path
import platform
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
    version = args.version.replace("+alpha.", "-alpha.")
    arch = platform.machine().lower()
    arch = {"amd64": "x86_64", "aarch64": "arm64"}.get(arch, arch)
    prefix = f"furnish_{version}_{args.system}-{arch}"
    args.output.mkdir(parents=True, exist_ok=True)
    bin_dir = {"linux": root / "bin", "macos": root / "Contents/MacOS", "windows": root}[args.system]
    ext = ".exe" if args.system == "windows" else ""
    files = [bin_dir / (name + ext) for name in ("furnish", "furnish-gui")]
    repo = Path(__file__).resolve().parents[1]
    files.extend(repo / name for name in ("LICENSE", "THIRD-PARTY-LICENSES.html"))
    if args.system == "windows":
        with ZipFile(args.output / f"{prefix}_binaries.zip", "w", compression=ZIP_DEFLATED) as out:
            for file in files:
                out.write(file, file.name)
    else:
        with tarfile.open(args.output / f"{prefix}_binaries.tar.gz", "w:gz") as out:
            for file in files:
                out.add(file, arcname=file.name)
    if args.system == "linux":
        with tarfile.open(args.output / f"{prefix}_user.tar.gz", "w:gz") as out:
            for name in ("bin", "lib/furnish", "share/applications", "share/icons"):
                out.add(root / name, arcname="furnish/" + name)
            out.add(repo / "LICENSE", arcname="furnish/lib/furnish/LICENSE")


if __name__ == "__main__":
    main()
