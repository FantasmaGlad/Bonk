#!/usr/bin/env python3
"""Usage: release-gitlab.py <tag vX.Y.Z> <directory of the signed release files>"""
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

PROJET = "FantasmaGlad/Bonk"
API = "https://gitlab.com/api/v4/projects/" + urllib.parse.quote(PROJET, safe="")
ATTENTE_TAG = 20


def appel(methode: str, url: str, donnees: bytes | None = None, type_contenu: str = "application/json"):
    requete = urllib.request.Request(url, data=donnees, method=methode, headers={
        "PRIVATE-TOKEN": os.environ["GITLAB_TOKEN"], "Content-Type": type_contenu})
    with urllib.request.urlopen(requete, timeout=600) as reponse:
        return json.loads(reponse.read() or b"null")


def existe(url: str) -> bool:
    try:
        appel("GET", url)
        return True
    except urllib.error.HTTPError as erreur:
        if erreur.code == 404:
            return False
        raise


# The mirror workflow copies the tag when it is pushed; it may still be running.
def attendre_tag(tag: str) -> None:
    for _ in range(ATTENTE_TAG):
        if existe(f"{API}/repository/tags/{tag}"):
            return
        time.sleep(30)
    sys.exit(f"ERREUR : le tag {tag} n'est pas sur GitLab")


def notes(tag: str, empreinte: str, corps: str) -> str:
    changements = corps.split("## Download")[0].strip()
    return f"""{changements}

Mirror of https://github.com/{PROJET}/releases/tag/{tag}, same files.

## Verify
`SHA256SUMS` covers the binaries built by the CI, `SHA256SUMS-distribution` the package manifests. Both are
signed by the Bonk release signing key, fingerprint `{empreinte}`, given as `bonk-release-key.asc`.

```
gpg --import bonk-release-key.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
gpg --verify SHA256SUMS-distribution.asc SHA256SUMS-distribution
sha256sum --check SHA256SUMS SHA256SUMS-distribution
```

Provenance of the binaries, as built on GitHub: `gh attestation verify <file> --repo {PROJET}`.
"""


def main() -> None:
    tag, dossier = sys.argv[1], Path(sys.argv[2])
    version = tag.removeprefix("v")
    if existe(f"{API}/releases/{tag}"):
        sys.exit(f"ERREUR : la release {tag} existe déjà sur GitLab")
    attendre_tag(tag)
    liens = []
    for fichier in sorted(dossier.iterdir()):
        url = f"{API}/packages/generic/bonk/{version}/{fichier.name}"
        appel("PUT", url, fichier.read_bytes(), "application/octet-stream")
        liens.append({"name": fichier.name, "url": url, "link_type": "package"})
        print("envoyé", fichier.name)
    release = appel("POST", f"{API}/releases", json.dumps({
        "tag_name": tag, "name": f"Bonk {version}",
        "description": notes(tag, os.environ["EMPREINTE"], os.environ["NOTES"]),
        "assets": {"links": liens}}).encode())
    print("release", release["tag_name"], release["_links"]["self"], len(release["assets"]["links"]), "fichiers")


if __name__ == "__main__":
    main()
