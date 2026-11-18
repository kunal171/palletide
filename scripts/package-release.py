"""Package native release binaries on Linux, macOS and Windows."""
import hashlib
import os
from pathlib import Path
import tarfile

version = os.environ["GITHUB_REF_NAME"]
target = os.environ["TARGET"]
suffix = ".exe" if "windows" in target else ""
output = Path("dist")
output.mkdir(exist_ok=True)
checksums = []
for label, binary in [("codegen", "palletide-codegen"), ("transfers", "transfers")]:
    source = Path("target") / target / "release" / (binary + suffix)
    archive = output / f"palletide-{label}-{version}-{target}.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        tar.add(source, arcname=source.name)
    checksums.append(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
(output / f"SHA256SUMS-{target}").write_text("".join(checksums))
