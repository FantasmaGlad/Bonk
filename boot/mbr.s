# MBR de Bonk, provisoire : affiche qu'aucun code d'amorçage BIOS n'est encore posé, puis s'arrête.
# Chargé par le BIOS à 0000:7C00 ; 440 octets au plus, la signature du disque et la table suivent.

.pushsection .bonk_mbr, "ax"
.code16
mbr:
    cli
    xor ax, ax
    mov ds, ax
    mov ss, ax
    mov sp, 0x7c00
    sti
    cld
    mov si, offset message
print:
    lodsb
    test al, al
    jz stop
    mov ah, 0x0e
    mov bx, 0x0007
    int 0x10
    jmp print
stop:
    cli
    hlt
    jmp stop
message:
    .asciz "Bonk: this drive has no BIOS boot code\r\n"
    .fill 440 - (. - mbr), 1, 0
.code64
.popsection
