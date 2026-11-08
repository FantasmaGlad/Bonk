/* Adresse où le BIOS ou l'étage précédent charge chaque code : les adresses absolues y sont résolues. */
SECTIONS
{
    .bonk_mbr 0x7c00 : { KEEP(*(.bonk_mbr)) }
}
