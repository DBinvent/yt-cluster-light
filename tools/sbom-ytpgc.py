#!/usr/bin/env python3
"""[sbom] The extension's SBOM, CycloneDX 1.5, for .github/workflows/sbom.yml.

ytpgc is C with no package manifest a tool reads, so its components are what the built
library links (readelf's NEEDED entries) and the PostgreSQL it is built for, each named by
the Debian package that ships it where it was built, with that package's version (dpkg).

    sbom-ytpgc.py <yt_cluster .so> <VERSION file> <PostgreSQL major> <out.cdx.json>
"""
import datetime
import json
import os
import re
import subprocess
import sys
import urllib.parse
import uuid


def run(*cmd, required=False):
    """the command's stdout, "" when it fails - or, `required`, the run stops with its error
    ([review 2026-11-17]: a failing readelf or ldconfig showed only later, as "no package")"""
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0 and required:
        sys.exit(f"sbom-ytpgc: {' '.join(cmd)} failed: {r.stderr.strip()}")
    return r.stdout if r.returncode == 0 else ""


def owner(path):
    """the package that ships `path`, none when dpkg knows no package for it; a -dev
    package is not the runtime's"""
    for line in run("dpkg", "-S", path).splitlines():
        # [review 2026-11-17] a diversion's line names no owner: "diversion by PKG from: ..."
        if line.startswith("diversion by"):
            continue
        pkg = line.split(":", 1)[0].strip()
        if pkg and not pkg.endswith("-dev") and "," not in pkg:
            return pkg
    return None


def package_of(soname):
    """the package of a linked library: by its path as the loader finds it, that path with
    /lib and /usr/lib swapped (a merged /usr), then by its name anywhere"""
    paths = re.findall(r"\s" + re.escape(soname) + r" \([^)]*x86-64[^)]*\) => (\S+)", run("ldconfig", "-p", required=True))
    tries = []
    for p in paths:
        tries += [p, os.path.realpath(p)]
        tries.append(p[4:] if p.startswith("/usr/lib/") else "/usr" + p if p.startswith("/lib/") else p)
    # last, a pattern: with a wildcard in it dpkg -S matches whole paths, not a part of one
    tries.append("*/" + soname)
    for t in tries:
        pkg = owner(t)
        if pkg:
            return pkg
    return None


def distro():
    """the purl's distro qualifier, as /etc/os-release names this system: ubuntu-24.04"""
    rel = {}
    for line in open("/etc/os-release").read().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            rel[k] = v.strip('"')
    return rel.get("ID", "ubuntu") + "-" + rel.get("VERSION_ID", "")


def licenses_of(pkg):
    """[sbom] the package's licenses, as its Debian copyright file names them: the License
    fields of a machine-readable one (curl, BSD-2-clause, PostgreSQL ...), each once; a file
    in free form (glibc's) is named by its path - an SBOM lists every project with its license
    (owner, 2026-11-17)"""
    path = f"/usr/share/doc/{pkg}/copyright"
    try:
        text = open(path, errors="replace").read()
    except OSError:
        return []
    names = []
    # the license of the package's own files: the "Files: *" paragraph's - the others are of
    # parts of its source (autoconf's scripts and the like), not of the library
    for para in re.split(r"\n\s*\n", text):
        if re.search(r"^Files:\s*\*\s*$", para, re.M):
            m = re.search(r"^License:[ \t]*(\S[^\n]*)$", para, re.M)
            if m:
                return [{"license": {"name": m.group(1).strip()}}]
    for m in re.finditer(r"^License:[ \t]*(\S[^\n]*)$", text, re.M):
        n = m.group(1).strip()
        if n and n not in names:
            names.append(n)
    if names:
        return [{"license": {"name": n}} for n in names]
    # [review 2026-11-17] a file in free form (glibc's): the license it says, as SPDX - or none,
    # and the run stops on it (a "see the file" name passed the license gate)
    flat = " ".join(text.split())
    if re.search(r"GNU Lesser General Public License.{0,200}?version 2\.1 of the License, or \(at your option\) any later version", flat):
        return [{"expression": "LGPL-2.1-or-later"}]
    return []


def component(pkg, sonames=()):
    ver = run("dpkg-query", "-W", "-f=${Version}", pkg).strip()
    if not ver:
        sys.exit(f"sbom-ytpgc: no version of {pkg} installed")
    arch = run("dpkg-query", "-W", "-f=${Architecture}", pkg).strip() or "amd64"
    # [review 2026-11-17] the version percent-encoded (an epoch's ":", a "+"), and the distro
    purl = f"pkg:deb/ubuntu/{pkg}@{urllib.parse.quote(ver, safe='')}?arch={arch}&distro={distro()}"
    c = {"type": "library", "bom-ref": purl, "name": pkg, "version": ver, "purl": purl,
         "licenses": licenses_of(pkg)}
    if sonames:
        c["properties"] = [{"name": "yt:soname", "value": s} for s in sonames]
    return c


def main():
    if len(sys.argv) != 5:
        sys.exit(__doc__.strip().splitlines()[-1].strip())  # [review 2026-11-17] the usage, not a ValueError
    so, version_file, pg, out = sys.argv[1:5]
    version = open(version_file).read().strip()
    needed = re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", run("readelf", "-d", so, required=True))
    if not needed:
        sys.exit(f"sbom-ytpgc: no NEEDED entries read from {so}")
    # [review 2026-11-17] one component a package, with every library of it the extension
    # links (libc6 ships libc.so.6 and libm.so.6): a bom-ref is unique in a CycloneDX BOM
    by_pkg, missing = {}, []
    for soname in needed:
        pkg = package_of(soname)
        if pkg:
            by_pkg.setdefault(pkg, []).append(soname)
        else:
            missing.append(soname)
    comps = [component(pkg, sonames) for pkg, sonames in by_pkg.items()]
    server = f"postgresql-{pg}"
    if server not in by_pkg and run("dpkg-query", "-W", "-f=${Version}", server).strip():
        comps.append(component(server))
    if missing:
        # a library no package ships here is not left out quietly
        sys.exit(f"sbom-ytpgc: no package found for {', '.join(missing)}")
    root = "pkg:generic/yt_cluster@" + version
    bom = {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": f"urn:uuid:{uuid.uuid4()}",
        "version": 1,
        "metadata": {
            "timestamp": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "component": {
                "type": "library", "bom-ref": root, "name": "yt_cluster", "version": version,
                "description": f"YT Cluster's PostgreSQL extension (ytpgc), as built for PostgreSQL {pg}",
                "licenses": [{"license": {"id": "MIT"}}],
                "purl": root,
            },
        },
        "components": comps,
        "dependencies": [{"ref": root, "dependsOn": [c["bom-ref"] for c in comps]}]
            + [{"ref": c["bom-ref"], "dependsOn": []} for c in comps],
    }
    with open(out, "w") as f:
        json.dump(bom, f, indent=2)
        f.write("\n")
    print(f"{out}: yt_cluster {version}, {len(comps)} components")


if __name__ == "__main__":
    main()
