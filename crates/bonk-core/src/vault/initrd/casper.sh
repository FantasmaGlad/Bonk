#!/bin/sh
# Stands in for casper's scripts/casper-premount/20iso_scan, which GRUB replaces in Ubuntu's initrd. casper's
# initrd has no exFAT driver: instead of finding the ISO in the file system, this hands casper the BonkVault
# partition and the offset of the ISO in it (bonk.offset, written by `bonk vault add`), which casper maps as a
# loop device (LIVEMEDIA_OFFSET). Written against casper 27.04.1: a casper that drops LIVEMEDIA_OFFSET,
# wait_for_devs or /conf/param.conf, or moves this step, stops at its own "Unable to find a medium".

PREREQ=""

prereqs()
{
    echo "$PREREQ"
}

case $1 in
    prereqs)
        prereqs
        exit 0
        ;;
esac

. /scripts/casper-functions
. /scripts/casper-helpers

offset=
for x in $(cat /proc/cmdline); do
    case ${x} in
        bonk.offset=*)
            offset=${x#bonk.offset=}
            ;;
    esac
done
if [ "$offset" ]; then
    wait_for_devs
    # A USB drive can show up after udev settles: casper itself looks for its medium for a minute.
    tries=0
    until vault=$(/sbin/blkid -L BonkVault) || [ "$tries" -ge 60 ]; do
        sleep 1
        tries=$((tries + 1))
    done
    if [ "$vault" ]; then
        echo "LIVEMEDIA=${vault}" >> /conf/param.conf
        echo "LIVEMEDIA_OFFSET=${offset}" >> /conf/param.conf
    else
        panic "
No BonkVault partition found
"
    fi
fi
