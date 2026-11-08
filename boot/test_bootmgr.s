# BOOTMGR d'essai, jamais posé sur une clé. Le banc d'essai écrit sa taille en 4, le numéro de chaque bloc
# de 4 Kio à son début et « BONKEND! » à la fin : tout le fichier doit être arrivé dans l'ordre.

.pushsection .bonk_test_bootmgr, "ax"
.code16
tb:
    .byte 0xeb, 0x06
    .fill 6, 1, 0
    mov ax, cs
    mov ds, ax
    cmp ax, 0x2000
    jne tb_bad
    cmp dl, 0x80
    jne tb_bad
    xor ax, ax
    mov es, ax
    cmp word ptr es:[0x7dfe], 0xaa55
    jne tb_bad
    mov edx, dword ptr [4]
    sub edx, 8
    mov ecx, 1
tb_block:
    mov ebx, ecx
    shl ebx, 12
    add ebx, 2
    cmp ebx, edx
    ja tb_end
    mov ebx, ecx
    shl ebx, 8
    add ebx, 0x2000
    mov es, bx
    cmp word ptr es:[0], cx
    jne tb_bad
    inc ecx
    jmp tb_block
tb_end:
    mov ebx, edx
    shr ebx, 4
    add ebx, 0x2000
    mov es, bx
    mov di, dx
    and di, 15
    mov si, offset tb_marker
    mov cx, 8
    repe cmpsb
    jne tb_bad
    mov si, offset tb_good
    jmp tb_print
tb_bad:
    mov ax, cs
    mov ds, ax
    mov si, offset tb_bad_text
tb_print:
    lodsb
    test al, al
    jz tb_halt
    mov ah, 0x0e
    mov bx, 0x0007
    int 0x10
    jmp tb_print
tb_halt:
    cli
    hlt
    jmp tb_halt
tb_marker:
    .ascii "BONKEND!"
tb_good:
    .asciz "BONK: bootmgr OK\r\n"
tb_bad_text:
    .asciz "BONK: bootmgr BAD\r\n"
    .fill 512 - (. - tb), 1, 0
.code64
.popsection
