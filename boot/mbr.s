# Le BIOS charge ce secteur à 0000:7C00 ; il se recopie à 0000:0600 pour y lire le secteur de démarrage
# de la partition active. 440 octets au plus : la signature du disque et la table suivent.

.pushsection .bonk_mbr, "ax"
.code16
mbr:
    cli
    xor ax, ax
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov sp, 0x7c00
    cld
    mov si, 0x7c00
    mov di, 0x0600
    mov cx, 256
    rep movsw
    .byte 0xea
    .word relocated, 0
relocated:
    sti
    mov byte ptr [drive], dl
    mov si, 0x07be
    mov cx, 4
find:
    cmp byte ptr [si], 0x80
    je found
    add si, 16
    loop find
    mov si, offset no_active
    jmp fail
found:
    mov ah, 0x41
    mov bx, 0x55aa
    mov dl, byte ptr [drive]
    int 0x13
    jc no_lba_error
    cmp bx, 0xaa55
    jne no_lba_error
    test cl, 1
    jz no_lba_error
    mov eax, dword ptr [si + 8]
    mov dword ptr [dap_lba], eax
    push si
    mov si, offset dap
    mov ah, 0x42
    mov dl, byte ptr [drive]
    int 0x13
    pop si
    jc read_error
    cmp word ptr [0x7dfe], 0xaa55
    jne no_boot_sector
    mov dl, byte ptr [drive]
    .byte 0xea
    .word 0x7c00, 0
no_lba_error:
    mov si, offset no_lba
    jmp fail
read_error:
    mov si, offset bad_read
    jmp fail
no_boot_sector:
    mov si, offset no_boot
fail:
    lodsb
    test al, al
    jz halt
    mov ah, 0x0e
    mov bx, 0x0007
    int 0x10
    jmp fail
halt:
    cli
    hlt
    jmp halt
drive:
    .byte 0
dap:
    .byte 16, 0
    .word 1, 0x7c00, 0
dap_lba:
    .long 0, 0
no_active:
    .asciz "Bonk: no active partition\r\n"
no_lba:
    .asciz "Bonk: no LBA support\r\n"
bad_read:
    .asciz "Bonk: disk read error\r\n"
no_boot:
    .asciz "Bonk: no boot sector\r\n"
    .fill 440 - (. - mbr), 1, 0
.code64
.popsection
