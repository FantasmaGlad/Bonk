#!/bin/sh
# Stands in for casper's scripts/casper-premount/20iso_scan, which GRUB replaces in Ubuntu's initrd. Same
# search for iso-scan/filename, with exFAT added to the file systems casper looks into: the vault's data
# partition is exFAT, absent from casper's list. Written against casper 27.04.1: a casper that renames
# find_path, casper-functions or casper-helpers, or moves this step, stops at its "Could not find the ISO".

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

modprobe exfat 2> /dev/null

is_supported_fs()
{
    [ -z "${1}" ] && return 1
    case ${1} in
        exfat|ext2|ext3|ext4|xfs|jfs|reiserfs|vfat|ntfs|iso9660|btrfs|udf)
            return 0
            ;;
    esac
    return 1
}

iso_path=
for x in $(cat /proc/cmdline); do
    case ${x} in
        iso-scan/filename=*)
            iso_path=${x#iso-scan/filename=}
            ;;
    esac
done
if [ "$iso_path" ]; then
    if find_path "${iso_path}" /isodevice rw; then
        echo "LIVEMEDIA=${FOUNDPATH}" >> /conf/param.conf
        if [ -f "${FOUNDPATH}" ]; then
            echo "LIVEMEDIA_OFFSET=0" >> /conf/param.conf
        fi
    else
        panic "
Could not find the ISO $iso_path
"
    fi
fi
