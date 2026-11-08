# Secteur de démarrage d'essai, jamais posé sur une clé : vérifie que le MBR a passé la main avec DL
# et DS:SI sur l'entrée active de la table.

.pushsection .bonk_test_vbr, "ax"
.code16
test_vbr:
    mov di, offset test_bad
    cmp dl, 0x80
    jne test_verdict
    cmp byte ptr [si], 0x80
    jne test_verdict
    mov di, offset test_good
test_verdict:
    xor ax, ax
    mov ds, ax
    mov si, di
test_print:
    lodsb
    test al, al
    jz test_halt
    mov ah, 0x0e
    mov bx, 0x0007
    int 0x10
    jmp test_print
test_halt:
    cli
    hlt
    jmp test_halt
test_good:
    .asciz "BONK: handover OK\r\n"
test_bad:
    .asciz "BONK: handover BAD\r\n"
    .fill 510 - (. - test_vbr), 1, 0
    .byte 0x55, 0xaa
.code64
.popsection
