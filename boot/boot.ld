SECTIONS
{
    .bonk_mbr 0x0600 : { KEEP(*(.bonk_mbr)) }
    .bonk_test_vbr 0x7c00 : { KEEP(*(.bonk_test_vbr)) }
}
