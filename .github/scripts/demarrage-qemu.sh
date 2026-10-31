#!/usr/bin/env bash
# Démarrage d'une clé écrite par Bonk, sous QEMU. La clé est un fichier image : aucun périphérique réel n'est
# touché.
# Usage : demarrage-qemu.sh <bonk> <image ou URL> <bios|uefi|secureboot> <dossier de travail> <preuve> [options]
# Avec une URL : SHA256_FILE donne l'adresse du fichier d'empreintes. Options : celles de demarrage-qemu.py.
# Niveau de preuve : un motif du système démarré (installateur, invite de connexion) sur la console série,
# après écriture et relecture par Bonk.
set -euo pipefail

bonk=$1
source=$2
mode=$3
travail=$4
preuve=$5
shift 5
mkdir -p "$travail"
ici=$(cd "$(dirname "$0")" && pwd)

empreinte=()
if [ -n "${SHA256_FILE:-}" ]; then
  empreinte=(--sha256-file "$SHA256_FILE" --cache-dir "$travail/cache")
fi
"$bonk" write --simulate "$travail/cle.img" "$source" "${empreinte[@]}" --yes --progress none

chargeur=${CHARGEUR:-grub}
if [ "$mode" = bios ]; then
  chargeur=${CHARGEUR_BIOS:-$chargeur}
fi
python3 "$ici/demarrage-qemu.py" "$travail/cle.img" "$mode" "$chargeur" "$preuve" "$travail" "$@"
