# BIOS loads this sector at 0000:7C00 with DL = drive; the partition table follows these 440 bytes.

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
    # A key emulated as a floppy passes DL below 0x80: forcing 0x80 would read the internal disk.
    mov byte ptr [drive], dl
    mov si, 0x07be
    xor di, di
    mov cx, 4
scan:
    mov al, byte ptr [si]
    test al, al
    jz next_entry
    cmp al, 0x80
    jne invalid_table
    test di, di
    jnz invalid_table
    mov di, si
next_entry:
    add si, 16
    loop scan
    test di, di
    jz no_active_error
    cmp byte ptr [di + 4], 0
    je invalid_table
    mov eax, dword ptr [di + 8]
    test eax, eax
    jz invalid_table
    mov word ptr [entry], di
    mov dword ptr [dap_lba], eax
    mov ah, 0x41
    mov bx, 0x55aa
    mov dl, byte ptr [drive]
    int 0x13
    jc no_lba_error
    cmp bx, 0xaa55
    jne no_lba_error
    test cl, 1
    jz no_lba_error
    mov byte ptr [tries], 4
read:
    mov word ptr [dap_count], 1
    mov si, offset dap
    mov ah, 0x42
    mov dl, byte ptr [drive]
    int 0x13
    jnc loaded
    dec byte ptr [tries]
    jz read_error
    xor ah, ah
    mov dl, byte ptr [drive]
    int 0x13
    jmp read
loaded:
    cmp word ptr [0x7dfe], 0xaa55
    jne no_boot_error
    mov si, word ptr [entry]
    mov dl, byte ptr [drive]
    .byte 0xea
    .word 0x7c00, 0
invalid_table:
    mov si, offset bad_table
    jmp fail
no_active_error:
    mov si, offset no_active
    jmp fail
no_lba_error:
    mov si, offset no_lba
    jmp fail
read_error:
    mov si, offset bad_read
    jmp fail
no_boot_error:
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
tries:
    .byte 0
entry:
    .word 0
dap:
    .byte 16, 0
dap_count:
    .word 1
    .word 0x7c00, 0
dap_lba:
    .long 0, 0
bad_table:
    .asciz "Bonk: invalid partition table\r\n"
no_active:
    .asciz "Bonk: no active partition\r\n"
no_lba:
    .asciz "Bonk: no LBA support\r\n"
bad_read:
    .asciz "Bonk: disk read error\r\n"
no_boot:
    .asciz "Bonk: no boot sector\r\n"
    .org mbr + 440, 0
.code64
.popsection
