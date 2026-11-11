#!/usr/bin/env bash
# Usage: demarrage-vault.sh <bonk> <iso url> <SHA256SUMS url> <work dir> <proof> [demarrage-qemu.py options]
set -euo pipefail

bonk=$1
source=$2
sommes=$3
travail=$4
preuve=$5
shift 5
mkdir -p "$travail"
ici=$(cd "$(dirname "$0")" && pwd)
nom=${source##*/}
cle="$travail/cle.img"

"$bonk" vault init --simulate "$cle" --size "${VAULT_SIZE:-$((3 << 30))}" --yes --progress none --cache-dir "$travail/cache"

curl --fail --silent --show-error --location --retry 5 --retry-all-errors --continue-at - --output "$travail/$nom" "$source"
# GNU lines (`sum  name`) or BSD lines (`SHA256 (name) = sum`, Fedora).
attendu=$(curl --fail --silent --show-error --location "$sommes" | awk -v n="$nom" '
  $2 == n || $2 == "*" n {print $1}
  $1 == "SHA256" && $2 == "(" n ")" {print $4}')
test -n "$attendu"
echo "$attendu  $travail/$nom" | sha256sum --check --quiet

# COPIE=bonk: added by `bonk vault add`; otherwise as a user would put it there, by the system's exFAT driver.
if [ "${COPIE:-}" = bonk ]; then
  "$bonk" vault add --simulate "$cle" "$travail/$nom" --yes --progress none
elif sudo -n true 2> /dev/null; then
  boucle=$(sudo losetup --find --partscan --show "$cle")
  point=$(mktemp -d)
  sudo mount -t exfat "${boucle}p1" "$point"
  sudo cp "$travail/$nom" "$point/"
  sudo umount "$point"
  sudo losetup --detach "$boucle"
else
  boucle=$(udisksctl loop-setup --no-user-interaction --file "$cle" | grep -o '/dev/loop[0-9]*')
  # The desktop may have mounted it already: only a missing mount point is a failure.
  udisksctl mount --no-user-interaction --block-device "${boucle}p1" > /dev/null || true
  point=$(findmnt --noheadings --output TARGET "${boucle}p1") || {
    echo "ERREUR : ${boucle}p1 non monté" >&2
    exit 1
  }
  cp "$travail/$nom" "$point/"
  sync
  udisksctl unmount --no-user-interaction --block-device "${boucle}p1" > /dev/null
fi
"$bonk" vault list --simulate "$cle"

python3 "$ici/demarrage-qemu.py" "$cle" uefi grub "$preuve" "$travail" "$@"
