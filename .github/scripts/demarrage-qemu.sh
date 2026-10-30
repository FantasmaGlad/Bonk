#!/usr/bin/env bash
# Démarrage d'une clé écrite par Bonk, sous QEMU et OVMF. La clé est un fichier image : aucun
# périphérique réel n'est touché.
# Usage : demarrage-qemu.sh <bonk> <image.iso> <uefi|secureboot> <dossier de travail>
# Niveau de preuve : le micrologiciel UEFI trouve la clé, charge son chargeur d'amorçage et le lance
# (trace série « BdsDxe: starting »). Ce script ne vérifie pas que le système démarre jusqu'au bout.
set -euo pipefail

bonk=$1
iso=$2
mode=$3
travail=$4
mkdir -p "$travail"

"$bonk" write --simulate "$travail/cle.img" "$iso" --yes --progress none

ovmf=/usr/share/OVMF
case "$mode" in
  uefi)
    code="$ovmf/OVMF_CODE_4M.fd"
    cp "$ovmf/OVMF_VARS_4M.fd" "$travail/vars.fd"
    machine=q35
    extra=()
    ;;
  secureboot)
    code="$ovmf/OVMF_CODE_4M.secboot.fd"
    cp "$ovmf/OVMF_VARS_4M.ms.fd" "$travail/vars.fd"
    machine=q35,smm=on
    extra=(-global driver=cfi.pflash01,property=secure,value=on)
    ;;
  *) echo "mode inconnu : $mode" >&2; exit 2 ;;
esac

# `timeout` arrête la machine : le code 124 est attendu, le verdict vient de la trace série.
timeout 90 qemu-system-x86_64 -machine "$machine" "${extra[@]}" -m 1024 -nographic -no-reboot \
  -drive if=pflash,format=raw,readonly=on,file="$code" \
  -drive if=pflash,format=raw,file="$travail/vars.fd" \
  -drive if=none,id=cle,format=raw,file="$travail/cle.img",readonly=on \
  -device qemu-xhci -device usb-storage,drive=cle,bootindex=1 \
  > "$travail/serie.log" 2>&1 || true

if ! grep -aq 'BdsDxe: starting Boot[0-9A-F]* "UEFI QEMU QEMU USB HARDDRIVE' "$travail/serie.log"; then
  echo "ECHEC ($mode) : le micrologiciel n'a pas lancé le chargeur de la clé" >&2
  tail -n 30 "$travail/serie.log" >&2
  exit 1
fi
if grep -aqE 'Access Denied|Security Violation' "$travail/serie.log"; then
  echo "ECHEC ($mode) : démarrage refusé par le micrologiciel" >&2
  exit 1
fi
echo "OK ($mode) : le chargeur d'amorçage de la clé est lancé"
