#!/usr/bin/env bash
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
