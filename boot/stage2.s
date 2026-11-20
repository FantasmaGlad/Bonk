# Runs at 0000:7E00, BPB at 0000:7C00; starts BOOTMGR at 2000:0000 with DL = drive, as Windows boot code does.
# Memory: FAT sector at 0000:9000, directory cluster at 1000:0000, BOOTMGR up to the INT 12h top.

.pushsection .bonk_stage2, "ax"
.code16
stage2:
    cld
    mov byte ptr [s2_drive], dl
    cmp word ptr [0x7c0b], 512
    jne s2_unsupported
    movzx cx, byte ptr [0x7c0d]
    test cx, cx
    jz s2_unsupported
    mov ax, cx
    dec ax
    test ax, cx
    jnz s2_unsupported
    mov word ptr [s2_spc], cx
    cmp word ptr [0x7c0e], 17
    jb s2_unsupported
    cmp byte ptr [0x7c10], 0
    je s2_unsupported
    mov ebx, dword ptr [0x7c24]
    test ebx, ebx
    jz s2_unsupported

    # Mirroring off (bit 7): bits 0 to 3 name the active FAT.
    xor eax, eax
    mov dx, word ptr [0x7c28]
    test dl, 0x80
    jz s2_fat_chosen
    and dl, 0x0f
    cmp dl, byte ptr [0x7c10]
    jae s2_unsupported
    mov al, dl
s2_fat_chosen:
    mul ebx
    jc s2_unsupported
    movzx edx, word ptr [0x7c0e]
    add eax, edx
    jc s2_unsupported
    add eax, dword ptr [0x7c1c]
    jc s2_unsupported
    mov dword ptr [s2_fat_lba], eax

    movzx eax, byte ptr [0x7c10]
    mul ebx
    jc s2_unsupported
    movzx edx, word ptr [0x7c0e]
    add eax, edx
    jc s2_unsupported
    mov ecx, dword ptr [0x7c20]
    sub ecx, eax
    jbe s2_unsupported
    add eax, dword ptr [0x7c1c]
    jc s2_unsupported
    mov dword ptr [s2_data_lba], eax
    add eax, ecx
    jc s2_unsupported
    mov eax, ecx
    xor edx, edx
    movzx ecx, word ptr [s2_spc]
    div ecx
    test eax, eax
    jz s2_unsupported
    inc eax
    cmp eax, 0x0ffffff6
    jbe s2_last_in_range
    mov eax, 0x0ffffff6
s2_last_in_range:
    mov dword ptr [s2_last], eax
    shr eax, 7
    cmp eax, ebx
    jae s2_unsupported

    int 0x12
    movzx eax, ax
    shl eax, 1
    sub eax, 0x100
    jnc s2_memory_known
    xor eax, eax
s2_memory_known:
    mov dword ptr [s2_limit], eax

    # 65,536 entries at most per directory: 4,096 clusters of a single sector.
    mov word ptr [s2_dir_budget], 4096
    mov eax, dword ptr [0x7c2c]
s2_dir_cluster:
    call s2_check
    jc s2_volume_damaged
    dec word ptr [s2_dir_budget]
    jz s2_volume_damaged
    mov dword ptr [s2_cluster], eax
    mov cx, word ptr [s2_spc]
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
    call s2_check
    jc s2_damaged
    mov dword ptr [s2_cluster], eax
    mov eax, edx
    shr eax, 9
    test dx, 511
    jz s2_whole_sectors
    inc eax
s2_whole_sectors:
    cmp eax, dword ptr [s2_limit]
    ja s2_too_large
    mov dword ptr [s2_left], eax
    mov word ptr [s2_segment], 0x2000
s2_load:
    movzx ecx, word ptr [s2_spc]
    cmp ecx, dword ptr [s2_left]
    jbe s2_count_known
    mov ecx, dword ptr [s2_left]
s2_count_known:
    mov eax, dword ptr [s2_cluster]
    mov bx, word ptr [s2_segment]
    call s2_read_cluster
    movzx ecx, word ptr [s2_spc]
    sub dword ptr [s2_left], ecx
    jbe s2_start
    shl cx, 5
    add word ptr [s2_segment], cx
    mov eax, dword ptr [s2_cluster]
    call s2_next_cluster
    call s2_check
    jc s2_damaged
    mov dword ptr [s2_cluster], eax
    jmp s2_load
s2_start:
    xor ax, ax
    mov es, ax
    mov dl, byte ptr [s2_drive]
    .byte 0xea
    .word 0, 0x2000

s2_unsupported:
    mov si, offset s2_msg_unsupported
    jmp s2_fail
s2_volume_damaged:
    mov si, offset s2_msg_volume
    jmp s2_fail
s2_damaged:
    mov si, offset s2_msg_damaged
    jmp s2_fail
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

# EAX: cluster; carry set outside 2 to the last cluster of the volume.
s2_check:
    cmp eax, 2
    jb s2_check_done
    cmp dword ptr [s2_last], eax
s2_check_done:
    ret

# EAX: checked cluster; CX: its first sectors to read; BX: destination segment.
s2_read_cluster:
    sub eax, 2
    movzx edx, word ptr [s2_spc]
    imul eax, edx
    add eax, dword ptr [s2_data_lba]
s2_read_run:
    mov dx, cx
    cmp dx, 64
    jbe s2_run_sized
    mov dx, 64
s2_run_sized:
    mov word ptr [s2_count], dx
    mov dword ptr [s2_dap_lba], eax
    mov word ptr [s2_dap_segment], bx
    call s2_read
    movzx edx, word ptr [s2_count]
    add eax, edx
    sub cx, dx
    shl dx, 5
    add bx, dx
    test cx, cx
    jnz s2_read_run
    ret

# EAX: checked cluster; returns its FAT entry in EAX.
s2_next_cluster:
    mov ebx, eax
    shr eax, 7
    add eax, dword ptr [s2_fat_lba]
    and bx, 127
    shl bx, 2
    cmp eax, dword ptr [s2_fat_cached]
    je s2_fat_ready
    mov dword ptr [s2_fat_cached], 0xffffffff
    mov dword ptr [s2_dap_lba], eax
    mov word ptr [s2_count], 1
    mov word ptr [s2_dap_segment], 0x0900
    call s2_read
    mov dword ptr [s2_fat_cached], eax
s2_fat_ready:
    mov eax, dword ptr [0x9000 + bx]
    and eax, 0x0fffffff
    ret

# A BIOS may lower the DAP count to what it transferred: each try writes it again.
s2_read:
    pushad
    mov byte ptr [s2_tries], 4
s2_try:
    mov ax, word ptr [s2_count]
    mov word ptr [s2_dap_count], ax
    mov si, offset s2_dap
    mov ah, 0x42
    mov dl, byte ptr [s2_drive]
    int 0x13
    jnc s2_read_done
    dec byte ptr [s2_tries]
    jz s2_read_error
    xor ah, ah
    mov dl, byte ptr [s2_drive]
    int 0x13
    jmp s2_try
s2_read_done:
    popad
    ret

s2_drive:
    .byte 0
s2_tries:
    .byte 0
s2_spc:
    .word 0
s2_dir_budget:
    .word 0
s2_segment:
    .word 0
s2_count:
    .word 0
s2_fat_lba:
    .long 0
s2_data_lba:
    .long 0
s2_last:
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
s2_msg_unsupported:
    .asciz "Bonk: unsupported volume\r\n"
s2_msg_volume:
    .asciz "Bonk: volume damaged\r\n"
s2_msg_missing:
    .asciz "Bonk: BOOTMGR missing\r\n"
s2_msg_large:
    .asciz "Bonk: BOOTMGR too large\r\n"
s2_msg_damaged:
    .asciz "Bonk: BOOTMGR damaged\r\n"
s2_msg_read:
    .asciz "Bonk: disk read error\r\n"
    .org stage2 + 4092, 0
    .ascii "BNK2"
.code64
.popsection
