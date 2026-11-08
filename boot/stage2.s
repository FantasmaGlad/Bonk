# Second étage : trouve BOOTMGR dans le répertoire racine, le charge entier à 2000:0000 en suivant la FAT,
# puis lui passe la main avec DL, le secteur de démarrage restant à 0000:7C00.

.pushsection .bonk_stage2, "ax"
.code16
stage2:
    mov byte ptr [s2_drive], dl
    movzx eax, word ptr [0x7c0e]
    add eax, dword ptr [0x7c1c]
    mov dword ptr [s2_fat_lba], eax
    movzx ecx, byte ptr [0x7c10]
    mov ebx, dword ptr [0x7c24]
    imul ebx, ecx
    add eax, ebx
    mov dword ptr [s2_data_lba], eax
    movzx ax, byte ptr [0x7c0d]
    mov word ptr [s2_spc], ax
    int 0x12
    movzx eax, ax
    shl eax, 10
    sub eax, 0x20000
    mov dword ptr [s2_limit], eax
    mov eax, dword ptr [0x7c2c]
    mov word ptr [s2_dir_budget], 4096
s2_dir_cluster:
    dec word ptr [s2_dir_budget]
    jz s2_missing
    mov dword ptr [s2_cluster], eax
    mov bx, 0x1000
    call s2_read_cluster
    mov ax, 0x1000
    mov es, ax
    xor di, di
    mov cx, word ptr [s2_spc]
    shl cx, 4
s2_scan:
    cmp byte ptr es:[di], 0
    je s2_missing
    push cx
    push di
    mov si, offset s2_name
    mov cx, 11
    repe cmpsb
    pop di
    pop cx
    jne s2_next_entry
    test byte ptr es:[di + 11], 0x18
    jz s2_found
s2_next_entry:
    add di, 32
    loop s2_scan
    mov eax, dword ptr [s2_cluster]
    call s2_next_cluster
    cmp eax, 0x0ffffff8
    jb s2_dir_cluster
s2_missing:
    mov si, offset s2_msg_missing
    jmp s2_fail
s2_found:
    movzx eax, word ptr es:[di + 0x14]
    shl eax, 16
    mov ax, word ptr es:[di + 0x1a]
    mov edx, dword ptr es:[di + 0x1c]
    test edx, edx
    jz s2_missing
    mov dword ptr [s2_cluster], eax
    mov dword ptr [s2_left], edx
    movzx ebx, word ptr [s2_spc]
    shl ebx, 9
    add edx, ebx
    cmp edx, dword ptr [s2_limit]
    ja s2_too_large
    mov word ptr [s2_segment], 0x2000
s2_load:
    mov eax, dword ptr [s2_cluster]
    mov bx, word ptr [s2_segment]
    call s2_read_cluster
    mov ax, word ptr [s2_spc]
    shl ax, 5
    add word ptr [s2_segment], ax
    movzx ebx, word ptr [s2_spc]
    shl ebx, 9
    sub dword ptr [s2_left], ebx
    jbe s2_start
    mov eax, dword ptr [s2_cluster]
    call s2_next_cluster
    mov dword ptr [s2_cluster], eax
    cmp eax, 0x0ffffff8
    jb s2_load
    mov si, offset s2_msg_damaged
    jmp s2_fail
s2_start:
    xor ax, ax
    mov es, ax
    mov dl, byte ptr [s2_drive]
    .byte 0xea
    .word 0, 0x2000
s2_too_large:
    mov si, offset s2_msg_large
    jmp s2_fail
s2_read_error:
    mov si, offset s2_msg_read
s2_fail:
    xor ax, ax
    mov ds, ax
s2_print:
    lodsb
    test al, al
    jz s2_halt
    mov ah, 0x0e
    mov bx, 0x0007
    int 0x10
    jmp s2_print
s2_halt:
    cli
    hlt
    jmp s2_halt

# EAX : numéro de cluster ; BX : segment de destination.
s2_read_cluster:
    sub eax, 2
    movzx ecx, word ptr [s2_spc]
    imul eax, ecx
    add eax, dword ptr [s2_data_lba]
    mov dword ptr [s2_dap_lba], eax
    mov word ptr [s2_dap_count], cx
    mov word ptr [s2_dap_segment], bx
    jmp s2_read

# EAX : cluster ; rend le suivant dans EAX, d'après la première FAT.
s2_next_cluster:
    mov ebx, eax
    shr eax, 7
    add eax, dword ptr [s2_fat_lba]
    and bx, 127
    shl bx, 2
    push bx
    cmp eax, dword ptr [s2_fat_cached]
    je s2_fat_ready
    mov dword ptr [s2_fat_cached], eax
    mov dword ptr [s2_dap_lba], eax
    mov word ptr [s2_dap_count], 1
    mov word ptr [s2_dap_segment], 0x0900
    call s2_read
s2_fat_ready:
    pop bx
    mov eax, dword ptr [0x9000 + bx]
    and eax, 0x0fffffff
    ret

s2_read:
    push si
    mov si, offset s2_dap
    mov ah, 0x42
    mov dl, byte ptr [s2_drive]
    int 0x13
    pop si
    jc s2_read_error
    ret

s2_drive:
    .byte 0
s2_spc:
    .word 0
s2_dir_budget:
    .word 0
s2_segment:
    .word 0
s2_fat_lba:
    .long 0
s2_data_lba:
    .long 0
s2_limit:
    .long 0
s2_cluster:
    .long 0
s2_left:
    .long 0
s2_fat_cached:
    .long 0xffffffff
s2_dap:
    .byte 16, 0
s2_dap_count:
    .word 0
    .word 0
s2_dap_segment:
    .word 0
s2_dap_lba:
    .long 0, 0
s2_name:
    .ascii "BOOTMGR    "
s2_msg_missing:
    .asciz "Bonk: BOOTMGR missing\r\n"
s2_msg_large:
    .asciz "Bonk: BOOTMGR too large\r\n"
s2_msg_damaged:
    .asciz "Bonk: BOOTMGR damaged\r\n"
s2_msg_read:
    .asciz "Bonk: disk read error\r\n"
    .fill 4096 - (. - stage2), 1, 0
.code64
.popsection
