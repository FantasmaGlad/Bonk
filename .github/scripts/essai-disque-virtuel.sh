#!/bin/sh
# Écrit et vérifie un disque RAM ; le disque système doit rester refusé. Le runner est sudoer sans mot de passe :
# macOS ne donne l'accès brut aux disques qu'à root, et authopen demanderait un mot de passe à l'écran.
set -eu
cargo build -p bonk --locked
bonk="$PWD/target/debug/bonk"
image="${RUNNER_TEMP:-/tmp}/essai.img"
disque=$(hdiutil attach -nomount ram://131072 | awk '{print $1}')
trap 'hdiutil detach "$disque" >/dev/null 2>&1 || true' EXIT
dd if=/dev/urandom of="$image" bs=1048576 count=4 2>/dev/null
printf '\125\252' | dd of="$image" bs=1 seek=510 conv=notrunc 2>/dev/null

lire() {
  "$bonk" list --all --allow-virtual --json | python3 -c "
import json,sys
disques=json.load(sys.stdin)['disks']
$1"
}
id=$(lire "print(next(d['id'] for d in disques if d['path']=='$disque' and d['eligibility']['verdict']=='offered'))")
systeme=$(lire "s=[d for d in disques if any(p['is_system_volume'] for p in d['partitions'])]
assert len(s)==1 and s[0]['eligibility']['verdict']=='excluded', s
print(s[0]['id'])")

sudo "$bonk" write "$id" "$image" --allow-virtual --mode raw --yes
sudo "$bonk" verify "$id" "$image" --allow-virtual

# FAT32 par Bonk, exFAT par le système.
sudo "$bonk" format "$id" --fs fat32 --label ESSAI --allow-virtual --yes
sudo "$bonk" format "$id" --fs exfat --label ESSAI --allow-virtual --yes

code=0
sudo "$bonk" write "$systeme" "$image" --allow-virtual --mode raw --yes || code=$?
test "$code" -eq 3
