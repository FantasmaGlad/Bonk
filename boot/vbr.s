# Secteur de démarrage FAT32 : le BPB (octets 3 à 89) est posé par Bonk au formatage. Lit le second étage
# (8 secteurs à partir du secteur 9 du volume, dans la zone réservée) à 0000:7E00 et lui passe la main.

.pushsection .bonk_vbr, "ax"
.code16
vbr:
    .byte 0xeb, 0x58, 0x90
    .fill 0x5a - (. - vbr), 1, 0
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00
    sti
    cld
    mov byte ptr [vbr_drive], dl
    mov ah, 0x41
    mov bx, 0x55aa
    int 0x13
    jc vbr_no_lba
    cmp bx, 0xaa55
    jne vbr_no_lba
    test cl, 1
    jz vbr_no_lba
    mov eax, dword ptr [0x7c1c]
    add eax, 9
    mov dword ptr [vbr_dap_lba], eax
    mov si, offset vbr_dap
    mov ah, 0x42
    mov dl, byte ptr [vbr_drive]
    int 0x13
    jc vbr_read_error
    mov dl, byte ptr [vbr_drive]
    .byte 0xea
    .word 0x7e00, 0
vbr_no_lba:
    mov si, offset vbr_msg_lba
    jmp vbr_fail
vbr_read_error:
    mov si, offset vbr_msg_read
vbr_fail:
    lodsb
    test al, al
    jz vbr_halt
    mov ah, 0x0e
    mov bx, 0x0007
    int 0x10
    jmp vbr_fail
vbr_halt:
    cli
    hlt
    jmp vbr_halt
vbr_drive:
    .byte 0
vbr_dap:
    .byte 16, 0
    .word 8, 0x7e00, 0
vbr_dap_lba:
    .long 0, 0
vbr_msg_lba:
    .asciz "Bonk: no LBA support\r\n"
vbr_msg_read:
    .asciz "Bonk: disk read error\r\n"
    .fill 510 - (. - vbr), 1, 0
    .byte 0x55, 0xaa
.code64
.popsection
