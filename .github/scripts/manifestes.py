#!/usr/bin/env python3
"""Génère les manifestes de distribution (Scoop, Homebrew, AUR, winget) d'une release à partir de son fichier
SHA256SUMS. Rien n'est publié : les fichiers sont joints à la release, le mainteneur les soumet aux canaux.

Usage : manifestes.py <SHA256SUMS> <tag vX.Y.Z> <dossier de sortie>
"""
import json
import re
import sys
from pathlib import Path

DEPOT = "FantasmaGlad/Bonk"
DESCRIPTION = "Bootable media writer for USB drives and external disks, with a graphical and a command-line interface."
DESCRIPTION_FR = "Création de supports d'amorçage (clés USB et disques externes), interface graphique et ligne de commande."
LICENCE = "MIT"


def lire_empreintes(chemin: Path) -> dict[str, str]:
    """nom d'archive -> SHA-256."""
    empreintes = {}
    for ligne in chemin.read_text(encoding="utf-8").splitlines():
        m = re.fullmatch(r"([0-9a-f]{64}) [ *](\S+)", ligne.strip())
        if m:
            empreintes[m.group(2)] = m.group(1)
    return empreintes


class Release:
    def __init__(self, empreintes: dict[str, str], tag: str):
        self.tag = tag
        self.version = tag.removeprefix("v")
        self.empreintes = empreintes

    def archive(self, nom: str) -> str:
        ext = "zip" if nom.startswith("windows") else "tar.gz"
        return f"bonk-{self.version}-{nom}.{ext}"

    def url(self, nom: str) -> str:
        return f"https://github.com/{DEPOT}/releases/download/{self.tag}/{self.archive(nom)}"

    def sha(self, nom: str) -> str:
        archive = self.archive(nom)
        if archive not in self.empreintes:
            raise SystemExit(f"archive absente de SHA256SUMS : {archive}")
        return self.empreintes[archive]


def scoop(r: Release) -> str:
    def entree(nom: str) -> dict:
        return {"url": r.url(nom), "hash": r.sha(nom), "extract_dir": f"bonk-{r.version}-{nom}"}

    manifeste = {
        "version": r.version,
        "description": DESCRIPTION,
        "homepage": f"https://github.com/{DEPOT}",
        "license": LICENCE,
        "architecture": {
            "64bit": entree("windows-x86_64"),
            "arm64": entree("windows-aarch64"),
            "32bit": entree("windows-i686"),
        },
        "bin": "bonk.exe",
        "checkver": "github",
    }
    return json.dumps(manifeste, indent=2) + "\n"


def homebrew(r: Release) -> str:
    def bloc(nom: str) -> str:
        return f'      url "{r.url(nom)}"\n      sha256 "{r.sha(nom)}"\n'

    return f'''class Bonk < Formula
  desc "{DESCRIPTION}"
  homepage "https://github.com/{DEPOT}"
  version "{r.version}"
  license "{LICENCE}"

  on_macos do
    on_arm do
{bloc("macos-aarch64")}    end
    on_intel do
{bloc("macos-x86_64")}    end
  end

  on_linux do
    on_arm do
{bloc("linux-aarch64")}    end
    on_intel do
{bloc("linux-x86_64")}    end
  end

  def install
    bin.install "bonk"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/bonk --version")
  end
end
'''


def aur(r: Release) -> str:
    def source(arch: str, nom: str) -> str:
        return f'source_{arch}=("{r.url(nom)}")\nsha256sums_{arch}=("{r.sha(nom)}")\n'

    return f'''# Maintainer: FantasmaGlad
pkgname=bonk-bin
pkgver={r.version}
pkgrel=1
pkgdesc="{DESCRIPTION}"
arch=('x86_64' 'aarch64')
url="https://github.com/{DEPOT}"
license=('{LICENCE}')
provides=('bonk')
conflicts=('bonk')
{source("x86_64", "linux-x86_64")}{source("aarch64", "linux-aarch64")}
package() {{
  install -Dm755 "$srcdir/bonk-$pkgver-linux-$CARCH/bonk" "$pkgdir/usr/bin/bonk"
  install -Dm644 "$srcdir/bonk-$pkgver-linux-$CARCH/LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}}
'''


def winget(r: Release) -> dict[str, str]:
    ident = "FantasmaGlad.Bonk"
    entete = f"PackageIdentifier: {ident}\nPackageVersion: {r.version}\n"
    pied = "ManifestVersion: 1.12.0\n"
    fiche = (
        "PublisherUrl: https://github.com/FantasmaGlad\nPackageName: Bonk\nPackageUrl: https://bonk-os.app\n"
        f"License: {LICENCE}\nLicenseUrl: https://github.com/{DEPOT}/blob/main/LICENSE\n"
    )

    def installateur(arch: str, nom: str) -> str:
        return (
            f"  - Architecture: {arch}\n"
            f"    InstallerUrl: {r.url(nom)}\n"
            f"    InstallerSha256: {r.sha(nom).upper()}\n"
            "    NestedInstallerFiles:\n"
            f"      - RelativeFilePath: bonk-{r.version}-{nom}\\bonk.exe\n"
            "        PortableCommandAlias: bonk\n"
        )

    return {
        f"{ident}.yaml": entete + "DefaultLocale: en-US\nManifestType: version\n" + pied,
        f"{ident}.installer.yaml": entete
        + "InstallerType: zip\nNestedInstallerType: portable\nInstallers:\n"
        + installateur("x64", "windows-x86_64")
        + installateur("arm64", "windows-aarch64")
        + installateur("x86", "windows-i686")
        + "ManifestType: installer\n"
        + pied,
        f"{ident}.locale.en-US.yaml": entete
        + "PackageLocale: en-US\nPublisher: FantasmaGlad\n"
        + fiche
        + f"ShortDescription: {DESCRIPTION}\n"
        + "ManifestType: defaultLocale\n"
        + pied,
        f"{ident}.locale.fr-FR.yaml": entete
        + "PackageLocale: fr-FR\nPublisher: FantasmaGlad\n"
        + fiche
        + f"ShortDescription: {DESCRIPTION_FR}\n"
        + "ManifestType: locale\n"
        + pied,
    }


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit(__doc__)
    sums, tag, sortie = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise SystemExit(f"tag inattendu : {tag}")
    r = Release(lire_empreintes(sums), tag)
    sortie.mkdir(parents=True, exist_ok=True)
    (sortie / "bonk.json").write_text(scoop(r), encoding="utf-8")
    (sortie / "bonk.rb").write_text(homebrew(r), encoding="utf-8")
    (sortie / "PKGBUILD").write_text(aur(r), encoding="utf-8")
    for nom, contenu in winget(r).items():
        (sortie / nom).write_text(contenu, encoding="utf-8")
    print(f"manifestes écrits dans {sortie}")


if __name__ == "__main__":
    main()
