# Bonk writes the FAT32 BPB over bytes 3 to 89; the second stage lies in volume sectors 9 to 16.

.pushsection .bonk_vbr, "ax"
.code16
vbr:
    .byte 0xeb, 0x58, 0x90
    .org vbr + 0x5a, 0
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
    jc vbr_missing
    mov dword ptr [vbr_dap_lba], eax
    mov byte ptr [vbr_tries], 4
vbr_read:
    mov word ptr [vbr_dap_count], 8
    mov si, offset vbr_dap
    mov ah, 0x42
    mov dl, byte ptr [vbr_drive]
    int 0x13
    jnc vbr_loaded
    dec byte ptr [vbr_tries]
    jz vbr_read_error
    xor ah, ah
    mov dl, byte ptr [vbr_drive]
    int 0x13
    jmp vbr_read
vbr_loaded:
    cmp dword ptr [0x7e00 + 4092], 0x324b4e42
    jne vbr_missing
    mov dl, byte ptr [vbr_drive]
    .byte 0xea
    .word 0x7e00, 0
vbr_no_lba:
    mov si, offset vbr_msg_lba
    jmp vbr_fail
vbr_missing:
    mov si, offset vbr_msg_missing
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
vbr_tries:
    .byte 0
vbr_dap:
    .byte 16, 0
vbr_dap_count:
    .word 8
    .word 0x7e00, 0
vbr_dap_lba:
    .long 0, 0
vbr_msg_lba:
    .asciz "Bonk: no LBA support\r\n"
vbr_msg_missing:
    .asciz "Bonk: second stage missing\r\n"
vbr_msg_read:
    .asciz "Bonk: disk read error\r\n"
    .org vbr + 510, 0
    .byte 0x55, 0xaa
.code64
.popsection
